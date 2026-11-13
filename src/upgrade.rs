//! Upgrading from the releases that had their own connectors for two networks (0.6.x). An account of one of those
//! networks named a connection of the old kind; the connection it needs now is a description, and a description's
//! tokens are never copied. So the upkeep makes the connection for it, from the description the package ships
//! (`connections:` in package.yaml, the same data as docs/connectors/; its `meta.carries` says which network and old
//! provider it replaces, and where the old connection kept its server), turned off, takes the one thing the old
//! connection held that is not a secret (the server's address) into it, links the account to it, and tells the
//! workspace's administrators once which accounts need connecting again and where. Nothing else is guessed: the secrets
//! (the client's secret, an app password) are the administrator's to add, and the connection's switch is theirs.
//!
//! An account is carried once: once it names a connection that is here it is not looked at again, so the message is sent
//! once however often the upkeep runs.
//!
//! (round five) A connection taken up from a description the package now ships reading any public host (`any_host`,
//! BD-65: the feeds) takes the new description when it is the earlier one but for its hosts, which an administrator
//! listed site by site: [`carry_any_host`]. One an administrator changed otherwise is left as it is (it works on the
//! hosts it lists).

use crate::obj;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::connections::ConnectionInput;
use ferrum_kernel::messages::{self, Message, Recipient};
use ferrum_kernel::{Ctx, Kernel, Result, Tx};
use serde_json::{Value, json};
use uuid::Uuid;

/// A name a connection may have, made from what an account is called.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c.to_ascii_lowercase() {
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ if !out.ends_with('_') => out.push('_'),
            _ => {}
        }
    }
    out.trim_matches('_').chars().take(40).collect()
}

/// A server's address, as an old connection's config wrote it, when it is one.
fn server(config: &Value, key: &str) -> Option<String> {
    let given = config.get(key)?.as_str()?.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(given).ok()?;
    (matches!(url.scheme(), "https" | "http") && url.host_str().is_some() && url.query().is_none() && url.path() == "/").then(|| given.to_string())
}

/// The shipped description of `network` with the server's address put where the placeholder was.
fn described(shipped: &Value, placeholder: &str, server: Option<&str>) -> Value {
    match server {
        Some(address) => serde_json::from_str(&shipped.to_string().replace(placeholder, address)).unwrap_or_else(|_| shipped.clone()),
        None => shipped.clone(),
    }
}

/// The parts of a description that say where it may call, which `any_host` replaces.
fn but_hosts(d: &Value) -> Value {
    let mut d = d.clone();
    if let Some(o) = d.as_object_mut() {
        for k in ["allowed_hosts", "any_host", "meta"] {
            o.remove(k);
        }
    }
    d
}

/// (round five) Connections of the earlier shape of a description now reading any public host take the shipped one.
pub(crate) async fn carry_any_host(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    let shipped: Vec<Value> = k.registry.catalog().package("social").map(|p| p.manifest.connections.clone()).unwrap_or_default();
    for c in shipped.iter().filter(|c| c["description"]["any_host"] == true) {
        let row: Option<(Uuid, String, bool, Value)> = sqlx::query_as("select id, label, enabled, description from conn where tenant_id = $1 and name = $2")
            .bind(ctx.tenant)
            .bind(c["name"].as_str().unwrap_or_default())
            .fetch_optional(&mut **tx)
            .await?;
        let Some((conn, label, enabled, old)) = row else { continue };
        if old["any_host"] == true || but_hosts(&old) != but_hosts(&c["description"]) {
            continue;
        }
        let input: ConnectionInput = serde_json::from_value(json!({"name": c["name"], "label": label, "description": c["description"], "enabled": enabled}))
            .map_err(ferrum_kernel::Error::internal)?;
        // One holding secrets an administrator gave it cannot read any host (BD-65): it stays as it is.
        if let Err(e) = k.save_connection_def(&elevated(ctx), Some(conn), &input).await
            && e.is_retryable()
        {
            return Err(e);
        }
    }
    Ok(())
}

/// Accounts of the networks the old releases had connectors for, whose connection is not here: each gets one,
/// turned off, from the shipped description, and the administrators are told once.
pub(crate) async fn carry_connections(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    let system = elevated(ctx);
    let shipped: Vec<Value> = k.registry.catalog().package("social").map(|p| p.manifest.connections.clone()).unwrap_or_default();
    let mut carried: Vec<Value> = Vec::new();
    for a in k.find_tx(&system, tx, "SocialAccount", &[], None, 1000).await? {
        let Some(account) = id(&a, "id") else { continue };
        if crate::accounts::reach(tx, ctx.tenant, s(&a, "connection")).await?.is_some() {
            continue;
        }
        let network = s(&a, "network").map(|n| n.trim().to_lowercase()).unwrap_or_default();
        let Some(description) = shipped.iter().find(|c| c["description"]["meta"]["carries"]["network"].as_str().is_some_and(|n| n.to_lowercase() == network))
        else {
            continue;
        };
        let carries = &description["description"]["meta"]["carries"];
        let (name, provider) = (description["name"].as_str().unwrap_or_default(), carries["provider"].as_str().unwrap_or_default());
        let (key, placeholder) = (carries["server"].as_str().unwrap_or_default(), carries["placeholder"].as_str().unwrap_or_default());
        // The server the old connection spoke to is not a secret; its token and password are, and stay where they are.
        let old: Option<Value> = match s(&a, "connection").and_then(|c| c.trim().parse::<Uuid>().ok()) {
            Some(old) => {
                sqlx::query_scalar("select config from connection where tenant_id = $1 and id = $2 and provider = $3")
                    .bind(ctx.tenant)
                    .bind(old)
                    .bind(provider)
                    .fetch_optional(&mut **tx)
                    .await?
            }
            None => None,
        };
        let address = old.as_ref().and_then(|c| server(c, key));
        let own = format!("{name}_{}", slug(s(&a, "handle").or(s(&a, "account_name")).unwrap_or("account")));
        let label = format!("{} ({})", description["label"].as_str().unwrap_or(name), s(&a, "account_name").unwrap_or_default());
        // One made by an earlier run that did not get as far as linking the account is the one.
        let made: Option<Uuid> =
            sqlx::query_scalar("select id from conn where tenant_id = $1 and name = $2").bind(ctx.tenant).bind(&own).fetch_optional(&mut **tx).await?;
        let conn = match made {
            Some(conn) => conn,
            None => {
                let input: ConnectionInput = serde_json::from_value(json!({
                    "name": own, "label": label, "description": described(&description["description"], placeholder, address.as_deref()), "enabled": false
                }))
                .map_err(ferrum_kernel::Error::internal)?;
                k.save_connection_def(&system, None, &input).await?
            }
        };
        k.set_values_tx(&system, tx, "SocialAccount", account, obj(json!({"connection": conn.to_string()}))).await?;
        carried.push(json!({"account": s(&a, "account_name"), "connection": own, "label": label}));
    }
    if carried.is_empty() {
        return Ok(());
    }
    let admins: Vec<Uuid> = sqlx::query_scalar("select id from app_user where tenant_id = $1 and is_admin and is_active and kind = 'member'")
        .bind(ctx.tenant)
        .fetch_all(&mut **tx)
        .await?;
    let message = Message {
        template: "social.reconnect".into(),
        to: admins.into_iter().map(Recipient::user).collect(),
        values: obj(json!({"count": carried.len(), "accounts": carried})),
        ..Default::default()
    };
    messages::send(k, tx, ctx, &message).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_old_connection_held_that_is_not_a_secret() {
        assert_eq!(server(&json!({"instance": " https://mastodon.social/ "}), "instance").as_deref(), Some("https://mastodon.social"));
        assert_eq!(server(&json!({"instance": "https://x.example/path"}), "instance"), None);
        assert_eq!(server(&json!({"instance": "not a url"}), "instance"), None);
        assert_eq!(server(&json!({}), "instance"), None);
        let shipped = json!({"base": "https://mastodon.example", "x": ["https://mastodon.example/oauth/authorize"]});
        assert_eq!(
            described(&shipped, "https://mastodon.example", Some("https://fosstodon.org")),
            json!({"base": "https://fosstodon.org", "x": ["https://fosstodon.org/oauth/authorize"]})
        );
        assert_eq!(slug("@North Wind.social"), "north_wind_social");
    }
}
