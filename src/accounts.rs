//! Accounts. An account is a connection: the workspace's sign-in at a provider, or a person's own,
//! through the platform's broker, or a key an administrator holds. Social keeps which connection
//! reaches the account, what its network takes in a post (a network profile), and what state the
//! connection is in: connected, needing to be connected again because the provider refused its
//! sign-in (the platform tells the owner; the account shows it), not connected, or turned off.
//!
//! What a connection offers is read from its description's operations, by what Social needs of
//! them (`publish`, `publish_media`, `delete`, `reply`, `send_message`, `metrics`, `account`):
//! no network is named here.

use crate::obj;
use crate::rules::Rules;
use async_trait::async_trait;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use uuid::Uuid;

/// Whether the workspace has any network profile: seeded when an administrator updates the app, so a workspace not
/// yet updated has none and is not asked for one.
pub(crate) async fn any_profile(hc: &mut HookCtx<'_>) -> Result<bool> {
    Ok(!hc.kernel.find_tx(&elevated(hc.ctx), hc.tx, "SocialProfile", &[], None, 1).await?.is_empty())
}

/// A connection as Social needs to see it.
pub(crate) struct Reach {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    desc: Value,
}

impl Reach {
    pub fn has(&self, operation: &str) -> bool {
        self.desc["operations"].get(operation).is_some()
    }

    /// Where its calls go.
    pub fn base(&self) -> &str {
        self.desc["base"].as_str().unwrap_or("\u{0}")
    }

    /// Whether an operation only reads (a GET, the default), so calls it without a key.
    pub fn reads(&self, operation: &str) -> bool {
        self.desc["operations"][operation]["method"].as_str().is_none_or(|m| m.eq_ignore_ascii_case("GET"))
    }

    /// The inputs an operation declares; none declared means it takes any.
    pub fn inputs(&self, operation: &str) -> Vec<String> {
        self.desc["operations"][operation]["inputs"].as_array().into_iter().flatten().filter_map(|i| i.as_str().map(str::to_string)).collect()
    }

    /// `input` filtered to what the operation declares it takes.
    pub fn fit(&self, operation: &str, input: Value) -> Value {
        let declared = self.inputs(operation);
        match input {
            Value::Object(o) if !declared.is_empty() => Value::Object(o.into_iter().filter(|(k, _)| declared.contains(k)).collect()),
            other => other,
        }
    }

    /// What the description's author keeps with it: its `meta`, never read by the platform.
    pub fn meta(&self) -> &Value {
        &self.desc["meta"]
    }

    /// What this network takes and asks, when the description says: `meta.social` (docs/connectors/README.md), else
    /// 0.7.0's `meta.capabilities` (named as a network profile's fields are). The connection is where a network's facts
    /// are kept, a server's own limit among them.
    pub fn social(&self, network: Option<&str>) -> Result<Option<Rules>> {
        let fallback = network.unwrap_or(&self.name);
        if let Some(v) = self.meta().get("social").filter(|v| v.is_object()) {
            return Rules::parse(v, fallback).map(Some).map_err(|e| Error::invalid(format!("the connection {}: {e}", self.name)));
        }
        let Some(caps) = self.meta()["capabilities"].as_object() else { return Ok(None) };
        Ok(Some(Rules::of_fields(caps, fallback)))
    }

    /// Whether it proves who calls by a sign-in (the authorisation-code flow, or a session) rather than a key.
    fn signs_in(&self) -> bool {
        matches!(self.desc["auth"]["type"].as_str(), Some("oauth" | "session"))
    }

    /// A login the platform makes itself from the connection's secrets (an app password), once and again as it expires.
    fn session(&self) -> bool {
        self.desc["auth"]["type"] == "session"
    }

    /// Each person's own account, rather than the workspace's.
    pub fn personal(&self) -> bool {
        self.signs_in() && self.desc["auth"]["owner"] == "person"
    }
}

/// The connection `which` (its id), as the workspace's connections hold it.
pub(crate) async fn reach(tx: &mut Tx, tenant: Uuid, which: Option<&str>) -> Result<Option<Reach>> {
    let Some(conn) = which.and_then(|w| w.trim().parse::<Uuid>().ok()) else { return Ok(None) };
    let row: Option<(Uuid, String, bool, Value)> = sqlx::query_as("select id, name, enabled, description from conn where tenant_id = $1 and id = $2")
        .bind(tenant)
        .bind(conn)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.map(|(id, name, enabled, desc)| Reach { id, name, enabled, desc }))
}

/// The user whose sign-in an account uses, by the email it names.
pub(crate) async fn person(tx: &mut Tx, tenant: Uuid, email: Option<&str>) -> Result<Option<Uuid>> {
    let Some(email) = email.map(str::trim).filter(|e| !e.is_empty()) else { return Ok(None) };
    Ok(sqlx::query_scalar("select id from app_user where tenant_id = $1 and lower(email::text) = lower($2)")
        .bind(tenant)
        .bind(email)
        .fetch_optional(&mut **tx)
        .await?)
}

/// A context for calling the account's connection: the system, as the person whose sign-in it uses
/// when it is each person's own.
pub(crate) async fn caller(k: &Kernel, tx: &mut Tx, tenant: Uuid, account: &Doc, reach: &Reach) -> Result<Ctx> {
    let mut ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    if reach.personal() {
        let user = person(tx, tenant, s(account, "connected_by")).await?;
        ctx.user = user.ok_or_else(|| {
            Error::invalid(format!(
                "{} is each person's own sign-in, and {} names nobody who connected it",
                reach.name,
                s(account, "account_name").unwrap_or("the account")
            ))
        })?;
    }
    Ok(ctx)
}

/// How the connection reaches the account now: (state, what to say about it, the account the provider names).
pub(crate) async fn state_of(tx: &mut Tx, tenant: Uuid, account: &Doc) -> Result<(&'static str, Option<String>, Option<String>)> {
    let Some(reach) = reach(tx, tenant, s(account, "connection")).await? else {
        return Ok(("Not Connected", Some("no connection by that id is here: connect this account again".into()), None));
    };
    if !reach.enabled {
        return Ok(("Connection Off", Some(format!("the connection {} is turned off", reach.name)), None));
    }
    if !reach.signs_in() {
        return Ok(("Connected", None, None));
    }
    let owner = if reach.personal() { person(tx, tenant, s(account, "connected_by")).await? } else { Some(Uuid::nil()) };
    let Some(owner) = owner else {
        return Ok(("Not Connected", Some(format!("{} is each person's own sign-in: say whose in Connected By", reach.name)), None));
    };
    // A session signs itself in on first use: until it has, there is nothing to say it is not.
    let held: Option<(String, Option<String>)> =
        sqlx::query_as("select status, account from conn_token where tenant_id = $1 and conn_id = $2 and user_id = $3")
            .bind(tenant)
            .bind(reach.id)
            .bind(owner)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(match held {
        None if reach.session() => ("Connected", None, None),
        None => ("Not Connected", Some("nobody has signed in to it yet".into()), None),
        Some((status, account)) if status == "connected" => ("Connected", None, account),
        Some(_) => ("Needs Reconnecting", Some("the provider no longer accepts the sign-in: connect the account again".into()), None),
    })
}

/// The account's state written onto it, when it changed.
pub(crate) async fn refresh(k: &Kernel, ctx: &Ctx, tx: &mut Tx, account: &Doc) -> Result<(&'static str, Option<String>)> {
    let (state, note, provider) = state_of(tx, ctx.tenant, account).await?;
    let changed = s(account, "state") != Some(state) || s(account, "state_note") != note.as_deref() || s(account, "provider_account") != provider.as_deref();
    if let (true, Some(account_id)) = (changed, id(account, "id")) {
        let values = obj(json!({"state": state, "state_note": note, "provider_account": provider}));
        k.set_values_tx(&elevated(ctx), tx, "SocialAccount", account_id, values).await?;
        // Told once, as it happens: whoever connected it, else the Social Managers.
        if state == "Needs Reconnecting" && s(account, "state") != Some(state) {
            let to = match person(tx, ctx.tenant, s(account, "connected_by")).await? {
                Some(u) => vec![ferrum_kernel::messages::Recipient::user(u)],
                None => crate::notices::holders(k, ctx, tx, account, "Social Manager").await?,
            };
            let accounts = json!([{"account": s(account, "account_name"), "label": s(account, "network")}]);
            crate::notices::send(k, ctx, tx, "reconnect", to, None, json!({"count": 1, "accounts": accounts, "reason": "refused"})).await?;
        }
    }
    Ok((state, note))
}

/// Every account's state, brought up to date (the upkeep).
pub(crate) async fn refresh_all(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    for a in k.find_tx(ctx, tx, "SocialAccount", &[], None, 1000).await? {
        refresh(k, ctx, tx, &a).await?;
    }
    Ok(())
}

/// What an account's network takes, found by name for an account that names none: the profile
/// called what the account's network is called.
async fn named(hc: &mut HookCtx<'_>, network: &str) -> Result<Option<Uuid>> {
    let found = hc.kernel.find_tx(&elevated(hc.ctx), hc.tx, "SocialProfile", &[Filter::eq("profile_name", network.trim())], None, 2).await?;
    Ok(match found.as_slice() {
        [one] => id(one, "id"),
        _ => None,
    })
}

/// Accounts that name no profile, linked to the one called what their network is (the upkeep, for
/// accounts made before profiles were records).
pub(crate) async fn carry_over(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    let system = elevated(ctx);
    for a in k.find_tx(&system, tx, "SocialAccount", &[], None, 1000).await? {
        let (Some(account), None, Some(network)) = (id(&a, "id"), id(&a, "profile"), s(&a, "network")) else { continue };
        let found = k.find_tx(&system, tx, "SocialProfile", &[Filter::eq("profile_name", network.trim())], None, 2).await?;
        if let [one] = found.as_slice() {
            k.set_values_tx(&system, tx, "SocialAccount", account, obj(json!({"profile": s(one, "id")}))).await?;
        }
    }
    Ok(())
}

pub struct AccountHook;

#[async_trait]
impl DocHook for AccountHook {
    fn name(&self) -> &'static str {
        "social.account"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["network", "profile", "connected_by", "state", "state_note", "provider_account"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if let Some(handle) = s(doc, "handle").map(|h| h.trim().trim_start_matches('@').to_string()) {
            doc.insert("handle".into(), json!(handle));
        }
        // The connection by its name, as Administration, Connections shows it, or by its id.
        if let Some(given) = s(doc, "connection").map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && c.parse::<Uuid>().is_err()) {
            let named: Vec<Uuid> = sqlx::query_scalar("select id from conn where tenant_id = $1 and lower(name) = lower($2) limit 2")
                .bind(hc.ctx.tenant)
                .bind(&given)
                .fetch_all(&mut **hc.tx)
                .await?;
            match named.as_slice() {
                [one] => {
                    doc.insert("connection".into(), json!(one.to_string()));
                }
                [] => return Err(Error::field("connection", format!("no connection is named {given}"))),
                _ => return Err(Error::field("connection", format!("more than one connection is named {given}: give its id"))),
            }
        }
        if id(doc, "profile").is_none()
            && let Some(network) = s(doc, "network").filter(|n| !n.trim().is_empty())
            && let Some(profile) = named(hc, network).await?
        {
            doc.insert("profile".into(), json!(profile.to_string()));
        }
        if s(doc, "network").is_none_or(|n| n.trim().is_empty())
            && let Some(profile) = id(doc, "profile")
        {
            let p = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialProfile", profile, false).await?;
            doc.insert("network".into(), json!(s(&p, "profile_name")));
        }
        // A person's own sign-in is the one saving the account's, unless it says whose.
        if s(doc, "connected_by").is_none_or(|c| c.trim().is_empty()) && !hc.ctx.email.is_empty() {
            let own = reach(hc.tx, hc.ctx.tenant, s(doc, "connection")).await?.is_some_and(|r| r.personal());
            if own {
                doc.insert("connected_by".into(), json!(hc.ctx.email.to_lowercase()));
            }
        }
        let (state, note, provider) = state_of(hc.tx, hc.ctx.tenant, doc).await?;
        doc.insert("state".into(), json!(state));
        doc.insert("state_note".into(), json!(note));
        doc.insert("provider_account".into(), json!(provider));
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let conn = s(doc, "connection").and_then(|c| c.trim().parse::<Uuid>().ok()).ok_or_else(|| Error::field("connection", "is a connection's id"))?;
        let Some(found) = reach(hc.tx, hc.ctx.tenant, Some(&conn.to_string())).await? else {
            return Err(Error::field("connection", "no connection by that id in this workspace"));
        };
        // A description that says what its network takes needs no profile; the profile is the default for one that does not.
        if id(doc, "profile").is_none() && found.social(None)?.is_none() && any_profile(hc).await? {
            return Err(Error::field("profile", "says what the network takes in a post: pick one of the network profiles"));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if let Some(out) = crate::picker::action(hc, name, doc, params).await? {
            return Ok(Some(out));
        }
        if !matches!(name, "connect" | "disconnect") {
            return Ok(None);
        }
        let conn = reach(hc.tx, hc.ctx.tenant, s(doc, "connection")).await?.ok_or_else(|| Error::invalid("no connection by that id is here"))?;
        match name {
            "connect" => {
                if !conn.signs_in() {
                    return Err(Error::invalid(format!("{} proves who calls with a key an administrator holds: there is nothing to sign in to", conn.name)));
                }
                if conn.session() {
                    // Signed in now, from its secrets: a wrong password shows here, not on the first post.
                    let signed = hc.kernel.connect_session(&elevated(hc.ctx), conn.id).await?;
                    let after = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", id(doc, "id").unwrap_or_default(), false).await?;
                    let (state, note) = refresh(hc.kernel, hc.ctx, hc.tx, &after).await?;
                    return Ok(Some(json!({"signed_in": signed, "state": state, "note": note})));
                }
                // The sign-in starts from the browser, signed in (the broker binds it to the session).
                Ok(Some(json!({"connect": {"method": "POST", "path": format!("/api/connections/{}/connect", conn.id)}})))
            }
            _ => {
                if !conn.signs_in() {
                    return Err(Error::invalid(format!("{} proves who calls with a key an administrator holds: there is nothing to sign out of", conn.name)));
                }
                // Tokens are deleted here and revoked at the provider where it gives a revoke address.
                let who = if conn.session() { elevated(hc.ctx) } else { hc.ctx.clone() };
                let revoked = hc.kernel.connect_remove(&who, conn.id).await?;
                let after = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", id(doc, "id").unwrap_or_default(), false).await?;
                let (state, note) = refresh(hc.kernel, hc.ctx, hc.tx, &after).await?;
                Ok(Some(json!({"revoked": revoked, "state": state, "note": note})))
            }
        }
    }
}
