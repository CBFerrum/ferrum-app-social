// When a post goes: not yet (a draft), at a time on the workspace's clock, at the next free posting time, or now; and
// whether it repeats (every N days, weeks or months, until a date or for a number of times). The server makes each
// repeat as a post of its own when the one before goes out, so nothing here invents later ones.

import { isolate, t, zoneLabel } from "@ferrum/client";
import { Field } from "@ferrum/kit";
import type { Draft } from "../draft";

export type Mode = "draft" | "time" | "queue" | "now";

/** What the person means by the time they have given: a time set is "at a time", else the draft's mode. */
export const modeOf = (d: Draft): Mode => (d.at ? "time" : "draft");

/** How a repeat ends, from what the draft holds. */
export const endOf = (d: Draft): "never" | "until" | "count" => (d.repeatUntil ? "until" : d.repeatCount ? "count" : "never");

export function When({ draft, mode, onMode, onChange, locked, note }: { draft: Draft; mode: Mode; onMode: (m: Mode) => void; onChange: (d: Draft) => void; locked?: boolean; note?: string }) {
  const end = endOf(draft);
  const repeats = !!draft.repeatEvery;
  const zone = zoneLabel();
  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });
  return (
    <fieldset className="card card-body stack" disabled={locked}>
      <legend className="section-title">{t("When")}</legend>
      <div className="segmented" role="group" aria-label={t("When it goes")}>
        {([["draft", t("Not yet")], ["time", t("At a time")], ["queue", t("Next free time")], ["now", t("Now")]] as [Mode, string][]).map(([m, label]) => (
          <button key={m} type="button" aria-pressed={mode === m} onClick={() => onMode(m)}>
            {label}
          </button>
        ))}
      </div>
      {mode === "time" && (
        <Field label={zone ? t("Goes out at ({zone})", { zone }) : t("Goes out at")} htmlFor="when-at">
          <input id="when-at" className="input" type="datetime-local" value={draft.at} onChange={(e) => set({ at: e.target.value })} />
        </Field>
      )}
      {mode === "queue" && <p className="muted">{t("It takes the next free posting time of its accounts; the time is shown once it is in the queue.")}</p>}
      {mode === "now" && <p className="muted">{t("It goes as soon as it is approved.")}</p>}
      {note && <p className="muted">{isolate(note)}</p>}
      <label className="row">
        <input type="checkbox" checked={repeats} onChange={(e) => set({ repeatEvery: e.target.checked ? "1" : "", repeatUntil: "", repeatCount: "" })} />
        {t("Repeat")}
      </label>
      {repeats && (
        <div className="row">
          <Field label={t("Every")} htmlFor="rep-every">
            <input id="rep-every" className="input" type="number" min={1} value={draft.repeatEvery} onChange={(e) => set({ repeatEvery: e.target.value })} />
          </Field>
          <Field label={t("Unit")} htmlFor="rep-unit">
            <select id="rep-unit" className="select" value={draft.repeatUnit} onChange={(e) => set({ repeatUnit: e.target.value as Draft["repeatUnit"] })}>
              <option value="Days">{t("Days")}</option>
              <option value="Weeks">{t("Weeks")}</option>
              <option value="Months">{t("Months")}</option>
            </select>
          </Field>
          <Field label={t("Ends")} htmlFor="rep-end">
            <select id="rep-end" className="select" value={end} onChange={(e) => set({ repeatUntil: e.target.value === "until" ? draft.repeatUntil || draft.at.slice(0, 10) : "", repeatCount: e.target.value === "count" ? draft.repeatCount || "5" : "" })}>
              <option value="never">{t("Never")}</option>
              <option value="until">{t("On a date")}</option>
              <option value="count">{t("After a number of times")}</option>
            </select>
          </Field>
          {end === "until" && (
            <Field label={t("Last day")} htmlFor="rep-until">
              <input id="rep-until" className="input" type="date" value={draft.repeatUntil} onChange={(e) => set({ repeatUntil: e.target.value })} />
            </Field>
          )}
          {end === "count" && (
            <Field label={t("Times in all")} htmlFor="rep-count">
              <input id="rep-count" className="input" type="number" min={2} value={draft.repeatCount} onChange={(e) => set({ repeatCount: e.target.value })} />
            </Field>
          )}
        </div>
      )}
    </fieldset>
  );
}
