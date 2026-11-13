// Notice preferences (/notices): which of the app's notices the signed-in person gets. A notice is on until the person has an
// opt-out for it (SocialNoticeOptOut: person, notice); turning it off makes the row, on again removes it.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { isolate, refusal, t } from "@ferrum/client";
import { Alert, ListSkeleton, PageHead, PageStack } from "@ferrum/kit";
import { useMe } from "@ferrum/records";
import { make, remove, type Row, rows, text } from "./data";
import { Await } from "./ui";

export const NOTICES = ["went_out", "failed", "review", "reconnect", "repeat_stopped", "digest"] as const;

/** What each notice is, in words. */
export function noticeWords(id: string): [string, string] {
  switch (id) {
    case "went_out":
      return [t("A post went out"), t("When a post of yours has gone out to its accounts.")];
    case "failed":
      return [t("A post did not go out"), t("When a post of yours could not be sent, with the reasons.")];
    case "review":
      return [t("A review needs you"), t("When a post waits for your decision, and when yours is decided.")];
    case "reconnect":
      return [t("An account needs reconnecting"), t("When an account's sign-in is no longer accepted.")];
    case "repeat_stopped":
      return [t("A repeat stopped"), t("When the next copy of a repeating post could not be made.")];
    default:
      return [t("The weekly digest"), t("A summary of the week's posts and numbers.")];
  }
}

export function NoticesRoute() {
  const client = useQueryClient();
  const email = ((useMe() as { data?: { user: { email: string } } }).data?.user.email ?? "").toLowerCase();
  const out = useQuery({ queryKey: ["social", "optouts", email], queryFn: () => rows("SocialNoticeOptOut", [["person", "=", email]]), enabled: !!email });
  const [busy, setBusy] = useState<string | null>(null);
  const [refused, setRefused] = useState("");
  const toggle = async (notice: string, row?: Row) => {
    setBusy(notice);
    setRefused("");
    try {
      if (row) await remove("SocialNoticeOptOut", row.id);
      else await make("SocialNoticeOptOut", { person: email, notice });
      await client.invalidateQueries({ queryKey: ["social", "optouts"] });
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <PageStack>
      <PageHead title={t("Notices")} description={t("Which notices of Social reach you.")} />
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      <Await q={out} skeleton={<ListSkeleton rows={4} />}>
        {(list) => (
          <ul className="stack" aria-label={t("Notices")}>
            {NOTICES.map((n) => {
              const row = list.find((r) => text(r.notice) === n);
              const [name, help] = noticeWords(n);
              return (
                <li key={n} className="card card-body row">
                  <label className="row spacer">
                    <input type="checkbox" checked={!row} disabled={busy === n} onChange={() => void toggle(n, row)} aria-label={noticeWords(n)[0]} aria-describedby={`notice-${n}`} />
                    <span>
                      <strong>{name}</strong>
                      <span id={`notice-${n}`} className="muted"> {help}</span>
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
        )}
      </Await>
    </PageStack>
  );
}
