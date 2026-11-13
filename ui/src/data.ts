// What every screen asks the platform: records by filter, one record, a create, a change, an action. Shapes are the
// platform's (docs/interface-notes.md of Social 0.8.0); nothing here knows a screen.

import { api, docPath, newKey, t } from "@ferrum/client";

export interface Row {
  id: string;
  version?: number;
  [k: string]: unknown;
}

/** `[field, op, value]`; a field may reach one level into a table (`targets.account`). */
export type Filter = [string, string, unknown];

/** A value as text: a string as it is, nothing as empty. */
export const text = (v: unknown): string => (typeof v === "string" ? v : v == null ? "" : String(v));

/** The rows of an entity the person may read, by filter (500 at most). */
export const rows = (entity: string, filters: Filter[] = [], extra: Record<string, string | number | boolean> = {}) =>
  api.get<{ rows: Row[] }>(docPath(entity), { filters: JSON.stringify(filters), limit: 500, ...extra }).then((r) => r.rows);

/** One record. */
export const record = (entity: string, id: string) => api.get<Row>(docPath(entity, id));

/** Makes a record; pressed twice it is made once. */
export const make = (entity: string, body: Record<string, unknown>) => api.post<Row>(docPath(entity), body, { idempotencyKey: newKey() });

/** Changes a record as it was read (its version), so a change made meanwhile is refused rather than overwritten. */
export const change = (entity: string, id: string, body: Record<string, unknown>, version?: number) =>
  api.patch<Row>(docPath(entity, id), body, { ifMatch: version, idempotencyKey: newKey() });

/** Deletes a record. */
export const remove = (entity: string, id: string) => api.del(docPath(entity, id));

/** Runs a record's action with its parameters; the answer is the action's result (docs/interface-notes.md section 3). */
export const run = <T = Row>(entity: string, id: string, action: string, body: Record<string, unknown> = {}) =>
  api.post<T>(docPath(entity, id, "actions", action), body, { idempotencyKey: newKey() });

/** The post statuses as stored, and the tone each is drawn in (stable ids, not labels). */
export const POST_TONES = { Draft: "neutral", "In Review": "amber", Approved: "green", Rejected: "red", Sending: "blue", Published: "green", Failed: "red" } as const;

/** SocialAccount.state as stored. */
export const STATE_TONES = { Connected: "green", "Needs Reconnecting": "amber", "Not Connected": "red", "Connection Off": "neutral" } as const;

/** The words of a stored state or status (the stored value is the id; the label is for reading). */
export function stateWord(state: string): string {
  switch (state) {
    case "Connected":
      return t("Connected");
    case "Needs Reconnecting":
      return t("Needs reconnecting");
    case "Not Connected":
      return t("Not connected");
    case "Connection Off":
      return t("Connection off");
    default:
      return state;
  }
}
export function statusWord(status: string): string {
  switch (status) {
    case "Draft":
      return t("Draft");
    case "In Review":
      return t("In review");
    case "Approved":
      return t("Approved");
    case "Rejected":
      return t("Sent back");
    case "Sending":
      return t("Sending");
    case "Published":
      return t("Published");
    case "Failed":
      return t("Failed");
    default:
      return status;
  }
}
