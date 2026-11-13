//! The workspace's own connections Social calls outside any account: picture and GIF sources, the address fetcher,
//! a link shortener, writing help, feeds. Each is named in Social Settings (its name, as Administration, Connections
//! shows it, or its id) and called as the workspace; what it is called is looked up by Social's name, or the name its
//! description's `meta.social.operations` maps that to.

use crate::accounts::{Reach, reach};
use ferrum_domain::util::{elevated, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The longest a service's `Retry-After` is heeded; past it Social asks again anyway.
pub(crate) const MOST_WAIT: u64 = 3600;

/// When each connection asked to be left alone until, by workspace (the upkeep's callers: numbers, plugs, clicks,
/// writing help, feeds; a job is put back by the jobs' runner instead).
// ponytail: kept in this process only; another replica may ask once more before it hears the service's own answer.
static WAITS: LazyLock<Mutex<HashMap<(Uuid, Uuid), Instant>>> = LazyLock::new(Mutex::default);

/// Seconds connection `conn` still asked to be left alone for, if it did.
pub(crate) fn waiting(tenant: Uuid, conn: Uuid) -> Option<u64> {
    let left = WAITS.lock().ok()?.get(&(tenant, conn)).and_then(|until| until.checked_duration_since(Instant::now()))?;
    Some(left.as_secs().max(1))
}

/// A call's outcome heeded: a `RetryAfter` (bounded by [`MOST_WAIT`]) keeps the connection's callers away until then.
pub(crate) fn heed<T>(tenant: Uuid, conn: Uuid, r: Result<T>) -> Result<T> {
    match r {
        Err(Error::RetryAfter(why, secs)) => {
            let secs = secs.clamp(1, MOST_WAIT);
            if let Ok(mut w) = WAITS.lock() {
                w.insert((tenant, conn), Instant::now() + Duration::from_secs(secs));
            }
            Err(Error::RetryAfter(why, secs))
        }
        other => other,
    }
}

/// Whether an error means "later" (a timeout, a limit, a 5xx, a service's Retry-After), not a refusal.
pub(crate) fn later(e: &Error) -> bool {
    matches!(e, Error::Unavailable(_) | Error::Wait(..) | Error::RetryAfter(..))
}

/// Social Settings, as the workspace holds them (empty when never saved).
pub(crate) async fn settings(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<Doc> {
    Ok(k.find_tx(&elevated(ctx), tx, "SocialSettings", &[], None, 1).await?.into_iter().next().unwrap_or_default())
}

/// The connection `given` names: its id, or its name.
pub(crate) async fn named(tx: &mut Tx, tenant: Uuid, given: &str) -> Result<Option<Reach>> {
    let given = given.trim();
    if given.is_empty() {
        return Ok(None);
    }
    if given.parse::<Uuid>().is_ok() {
        return reach(tx, tenant, Some(given)).await;
    }
    // ponytail: the platform has no call for a connection by name inside a transaction; read as accounts.rs does.
    let found: Vec<Uuid> = sqlx::query_scalar("select id from conn where tenant_id = $1 and lower(name) = lower($2) limit 2")
        .bind(tenant)
        .bind(given)
        .fetch_all(&mut **tx)
        .await?;
    match found.as_slice() {
        [one] => reach(tx, tenant, Some(&one.to_string())).await,
        _ => Ok(None),
    }
}

/// The connection a Social Settings field names, or a plain refusal saying which setting to fill.
pub(crate) async fn from_setting(k: &Kernel, ctx: &Ctx, tx: &mut Tx, field: &str, what: &str) -> Result<Reach> {
    let set = settings(k, ctx, tx).await?;
    let given = s(&set, field).unwrap_or_default().to_string();
    let found = named(tx, ctx.tenant, given.split(',').next().unwrap_or_default()).await?;
    found.ok_or_else(|| Error::invalid(format!("no connection is set for {what}: an administrator names one in Social Settings")))
}

/// Social's name of an operation, as this connection's description calls it.
pub(crate) fn op(conn: &Reach, name: &str) -> String {
    conn.meta()["social"]["operations"][name].as_str().unwrap_or(name).to_string()
}

/// Calls operation `name` of `conn` as the workspace, under `key` (needed past a GET), about a record when given.
pub(crate) async fn call(k: &Kernel, ctx: &Ctx, conn: &Reach, name: &str, inputs: Value, key: Option<String>, about: Option<(&str, Uuid)>) -> Result<Value> {
    let operation = op(conn, name);
    if !conn.has(&operation) {
        return Err(Error::invalid(format!("{} does not offer {name}", conn.name)));
    }
    if !conn.enabled {
        return Err(Error::invalid(format!("the connection {} is turned off", conn.name)));
    }
    if let Some(left) = waiting(ctx.tenant, conn.id) {
        return Err(Error::RetryAfter(format!("{} asked to wait", conn.name), left));
    }
    let system = Ctx::system(ctx.tenant, ctx.schema.clone());
    let c = Call {
        connection: conn.id.to_string(),
        operation: operation.clone(),
        inputs: conn.fit(&operation, inputs),
        key: if conn.reads(&operation) { None } else { key },
        entity: about.map(|(e, _)| e.to_string()),
        record: about.map(|(_, r)| r),
    };
    heed(ctx.tenant, conn.id, connections::call(k, &system, Some(crate::APP), &c).await.map_err(crate::said))
}

/// A JSON pointer's value, `""` the whole.
pub(crate) fn at(v: &Value, pointer: &str) -> Value {
    if pointer.is_empty() { v.clone() } else { v.pointer(pointer).cloned().unwrap_or(Value::Null) }
}

/// Whether the caller holds one of `roles` (or administers the workspace): for an app's own action, whose rights the
/// package's `roles:` cannot carry (the platform allows roles on configured actions only).
pub(crate) fn holds(ctx: &Ctx, roles: &[&str]) -> bool {
    ctx.is_admin || ctx.is_system || ctx.roles.iter().any(|r| roles.contains(&r.as_str()))
}
