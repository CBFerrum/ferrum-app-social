// A post's review: the request the platform's approval policy asked (its steps and decisions), the buttons for whoever may
// decide, the comments and decisions people outside gave through a review link, and the links themselves (given once,
// listed, revoked). Calls: GET .../approval, POST .../approval/decide, GET /api/approvals, SocialPost actions approve /
// reject / review_link, GET .../links, DELETE /api/links/{id}, SocialReview list.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api, docPath, formatMoment, isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Dialog, Field, toast } from "@ferrum/kit";
import { type Row, rows, run, text } from "../data";

export interface Request {
  id: string;
  status: "pending" | "approved" | "rejected" | "recalled" | "cancelled" | "failed";
  failure?: string | null;
  steps: { label?: string; state: "waiting" | "open" | "skipped" | "approved" | "rejected"; need: number | string; approvers: string[] }[];
  decisions: { step: number; decider: string; outcome: "approve" | "reject"; comment?: string | null; at: string; void?: boolean }[];
}
export interface Issued {
  id: string;
  actions: string[];
  uses: number;
  max_uses: number;
  expires_at: string;
  recipient?: string | null;
  revoked_at?: string | null;
  opened_at?: string | null;
}

const moment = (v?: string | null) => (v ? formatMoment(v, { dateStyle: "medium", timeStyle: "short" }) : "");

/** The words of a step's state. */
export function stepWord(state: string): string {
  switch (state) {
    case "open":
      return t("Waiting for a decision");
    case "approved":
      return t("Approved");
    case "rejected":
      return t("Sent back");
    case "skipped":
      return t("Skipped");
    default:
      return t("Not yet");
  }
}

/** Whether the person has this post in what they are to decide (`GET /api/approvals`). */
export function toDecide(mine: { to_decide?: { entity: string; record: string }[] } | undefined, id: string): boolean {
  return !!mine?.to_decide?.some((i) => i.entity === "SocialPost" && i.record === id);
}

function ReviewLink({ post, onClose }: { post: Row; onClose: () => void }) {
  const client = useQueryClient();
  const [v, setV] = useState({ email: "", name: "", decide: false, days: 7, uses: 20 });
  const [given, setGiven] = useState<{ url: string; sent_to?: string | null; expires_at: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");
  const make = async () => {
    setBusy(true);
    setRefused("");
    try {
      const out = await run<{ url: string; sent_to?: string | null; expires_at: string }>("SocialPost", post.id, "review_link", { ...v, email: v.email || undefined, name: v.name || undefined });
      setGiven(out);
      await client.invalidateQueries({ queryKey: ["social", "links", post.id] });
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(false);
    }
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(given!.url);
      toast(t("Copied"));
    } catch {
      toast(t("Copy it by hand: select the address"), "error");
    }
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title={t("Review link")}
      description={t("Someone outside can see how each account will get the post, comment, and, if you allow it, approve or ask for changes.")}
      footer={given ? <Button variant="primary" onClick={onClose}>{t("Done")}</Button> : (
        <>
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button variant="primary" loading={busy} onClick={() => void make()}>{t("Make the link")}</Button>
        </>
      )}
    >
      {refused && <Alert tone="error">{refused}</Alert>}
      {given ? (
        <div className="stack">
          <Alert tone="warning">{t("This address is shown once. Copy it now.")}</Alert>
          <input className="input mono" readOnly dir="ltr" value={given.url} aria-label={t("Review link")} onFocus={(e) => e.currentTarget.select()} />
          <div className="row">
            <Button onClick={() => void copy()} icon="copy">{t("Copy")}</Button>
            {given.sent_to && <span className="muted">{t("Mailed to {to}", { to: isolate(given.sent_to) })}</span>}
            <span className="muted">{t("Works until {at}", { at: moment(given.expires_at) })}</span>
          </div>
        </div>
      ) : (
        <div className="stack">
          <Field label={t("Send it to")} htmlFor="rl-email" hint={t("Leave empty to copy the link yourself.")}>
            <input id="rl-email" className="input" type="email" value={v.email} onChange={(e) => setV({ ...v, email: e.target.value })} />
          </Field>
          <Field label={t("Their name")} htmlFor="rl-name">
            <input id="rl-name" className="input" value={v.name} onChange={(e) => setV({ ...v, name: e.target.value })} />
          </Field>
          <label className="row">
            <input type="checkbox" checked={v.decide} onChange={(e) => setV({ ...v, decide: e.target.checked })} />
            {t("They may approve or ask for changes")}
          </label>
          <div className="row">
            <Field label={t("Works for (days)")} htmlFor="rl-days">
              <input id="rl-days" className="input" type="number" min={1} max={90} value={v.days} onChange={(e) => setV({ ...v, days: Number(e.target.value) })} />
            </Field>
            <Field label={t("Uses")} htmlFor="rl-uses">
              <input id="rl-uses" className="input" type="number" min={1} max={100} value={v.uses} onChange={(e) => setV({ ...v, uses: Number(e.target.value) })} />
            </Field>
          </div>
        </div>
      )}
    </Dialog>
  );
}

export function ReviewPanel({ post, onChanged }: { post: Row; onChanged: () => void }) {
  const client = useQueryClient();
  const id = post.id;
  const inReview = post.status === "In Review";
  const approval = useQuery({ queryKey: ["social", "approval", id, post.status], queryFn: () => api.get<Request | null>(docPath("SocialPost", id, "approval")), retry: false });
  const mine = useQuery({ queryKey: ["approvals"], queryFn: () => api.get<{ to_decide: { entity: string; record: string }[] }>("/api/approvals"), retry: false });
  const reviews = useQuery({ queryKey: ["social", "reviews", id], queryFn: () => rows("SocialReview", [["post", "=", id]], { sort: "at asc" }), retry: false });
  const links = useQuery({ queryKey: ["social", "links", id], queryFn: () => api.get<{ links: Issued[] }>(docPath("SocialPost", id, "links")).then((r) => r.links), retry: false });
  const [comment, setComment] = useState("");
  const [need, setNeed] = useState(false);
  const [refused, setRefused] = useState("");
  const [making, setMaking] = useState(false);
  const request = approval.data && approval.data.status === "pending" ? approval.data : null;
  const decide = useMutation({
    mutationFn: async (outcome: "approve" | "reject") => {
      if (request) return api.post(docPath("SocialPost", id, "approval", "decide"), { outcome, comment: comment.trim() || undefined });
      // No platform request is pending: the post's own approve / send back decides.
      return outcome === "approve" ? run("SocialPost", id, "approve") : run("SocialPost", id, "reject", { note: comment.trim() });
    },
    onSuccess: async () => {
      setComment("");
      await client.invalidateQueries({ queryKey: ["social"] });
      onChanged();
    },
    onError: (e) => setRefused(refusal(e)),
  });
  const go = (outcome: "approve" | "reject") => {
    setRefused("");
    if (outcome === "reject" && !comment.trim()) return setNeed(true);
    setNeed(false);
    decide.mutate(outcome);
  };
  const recall = useMutation({
    mutationFn: () => api.post(docPath("SocialPost", id, "approval", "recall"), {}),
    onSuccess: async () => {
      toast(t("The request is taken back"));
      await client.invalidateQueries({ queryKey: ["social"] });
      onChanged();
    },
    onError: (e) => setRefused(refusal(e)),
  });
  const revoke = async (l: Issued) => {
    if (!(await confirmDialog({ title: t("Revoke this link?"), message: t("Whoever has it can no longer open the post."), confirmLabel: t("Revoke"), danger: true }))) return;
    try {
      await api.del(`/api/links/${l.id}`);
      await client.invalidateQueries({ queryKey: ["social", "links", id] });
    } catch (e) {
      toast(refusal(e), "error");
    }
  };
  const decided = approval.data?.decisions.filter((d) => !d.void) ?? [];
  return (
    <section className="card card-body stack" aria-labelledby="review-h">
      <h2 id="review-h" className="section-title">{t("Review")}</h2>
      {approval.data && (
        <ol className="stack" aria-label={t("Steps")}>
          {approval.data.steps.map((s, i) => (
            <li key={i} className="row">
              <strong>{isolate(s.label || t("Step {n}", { n: String(i + 1) }))}</strong>
              <Badge tone={s.state === "approved" ? "green" : s.state === "rejected" ? "red" : s.state === "open" ? "amber" : "neutral"}>{stepWord(s.state)}</Badge>
            </li>
          ))}
        </ol>
      )}
      {request && (
        <div>
          <Button size="sm" loading={recall.isPending} onClick={() => recall.mutate()}>{t("Take the request back")}</Button>
        </div>
      )}
      {decided.length > 0 && (
        <ul className="stack" aria-label={t("Decisions")}>
          {decided.map((d, i) => (
            <li key={i}>
              {d.outcome === "approve" ? t("Approved") : t("Sent back")} · {moment(d.at)}
              {d.comment && <span className="muted"> · {isolate(d.comment)}</span>}
            </li>
          ))}
        </ul>
      )}
      {text(post.review_note) && <Alert tone="warning">{isolate(text(post.review_note))}</Alert>}
      {inReview && (request ? toDecide(mine.data, id) : true) && (
        <div className="stack">
          {refused && <Alert tone="error">{refused}</Alert>}
          <Field label={t("Your comment")} htmlFor="rv-comment" error={need ? t("Say why when you send it back") : undefined}>
            <textarea id="rv-comment" className="textarea" rows={2} value={comment} onChange={(e) => setComment(e.target.value)} />
          </Field>
          <div className="row">
            <Button variant="primary" loading={decide.isPending && decide.variables === "approve"} disabled={decide.isPending} onClick={() => go("approve")}>
              {request ? t("Approve") : t("Approve it")}
            </Button>
            <Button variant="danger" loading={decide.isPending && decide.variables === "reject"} disabled={decide.isPending} onClick={() => go("reject")}>
              {t("Send it back")}
            </Button>
          </div>
        </div>
      )}
      {(reviews.data ?? []).length > 0 && (
        <ul className="stack" aria-label={t("What reviewers said")}>
          {reviews.data!.map((r) => (
            <li key={r.id}>
              <strong>{isolate(text(r.reviewer_name) || text(r.reviewer))}</strong> · {text(r.outcome) === "Approved" ? t("Approved") : text(r.outcome) === "Changes Asked" ? t("Asked for changes") : t("Commented")} · {moment(text(r.at))}
              {text(r.text) && <div>{isolate(text(r.text))}</div>}
            </li>
          ))}
        </ul>
      )}
      <div className="row">
        <h3 className="section-title spacer">{t("Review links")}</h3>
        <Button size="sm" onClick={() => setMaking(true)}>{t("Make a review link")}</Button>
      </div>
      {(links.data ?? []).length === 0 ? (
        <p className="muted">{t("No link has been made for this post.")}</p>
      ) : (
        <ul className="stack">
          {links.data!.map((l) => (
            <li key={l.id} className="row">
              <span className="spacer">
                {l.recipient ? isolate(l.recipient) : t("Anyone with it")} · {t("until {at}", { at: moment(l.expires_at) })} · {l.uses}/{l.max_uses}
                {l.actions.some((a) => a !== "review_comment") && ` · ${t("may decide")}`}
              </span>
              {l.revoked_at ? <Badge>{t("Revoked")}</Badge> : <Button size="sm" variant="danger" onClick={() => void revoke(l)}>{t("Revoke")}</Button>}
            </li>
          ))}
        </ul>
      )}
      {making && <ReviewLink post={post} onClose={() => setMaking(false)} />}
    </section>
  );
}
