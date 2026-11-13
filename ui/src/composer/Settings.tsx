// What an account's network asks, drawn from its rules: the kind of post, then each setting that applies (text, long,
// number, bool, select, multi, date), required marked, defaults shown, choices fetched through the server's `choices`
// call with a search, settings that depend on another answer shown only when it holds. Nothing here names a network.

import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { isolate, t } from "@ferrum/client";
import { Field, useDebounced } from "@ferrum/kit";
import { run } from "../data";
import { asked, kindLabel, optionLabel, optionValue, type Rules, type Setting } from "../model";

export interface Choice {
  value: unknown;
  label: string;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** The choices a setting fetches (boards, subreddits, flairs …) for a search, given the answers so far. */
export function useChoices(account: string, s: Setting, answers: Record<string, unknown>, query: string, enabled = true) {
  const q = useDebounced(query, 250);
  const fetched = !!s.choices;
  return useQuery({
    queryKey: ["social", "choices", account, s.key, q, JSON.stringify(answers)],
    queryFn: () => run<{ choices: Choice[] }>("SocialAccount", account, "choices", { setting: s.key, query: q, settings: answers }).then((r) => r.choices),
    enabled: enabled && fetched,
    retry: false,
  });
}

function Choose({ account, s, value, answers, onChange, id }: { account: string; s: Setting; value: unknown; answers: Record<string, unknown>; onChange: (v: unknown) => void; id: string }) {
  const [search, setSearch] = useState("");
  const fetched = !!s.choices;
  const q = useChoices(account, s, answers, search);
  const options: Choice[] = fetched ? (q.data ?? []) : s.options.map((o) => ({ value: optionValue(o), label: optionLabel(o) }));
  const has = (v: unknown) => options.some((o) => same(o.value, v));
  if (s.type === "multi") {
    const chosen = Array.isArray(value) ? value : [];
    const shown = [...options, ...chosen.filter((c) => !has(c)).map((c) => ({ value: c, label: String(c) }))];
    return (
      <div className="stack" role="group" aria-labelledby={`${id}-label`}>
        {fetched && <input className="input" type="search" aria-label={t("Search")} placeholder={t("Search")} value={search} onChange={(e) => setSearch(e.target.value)} />}
        {fetched && q.isError && <p className="field-error">{(q.error as Error).message}</p>}
        {shown.map((o, i) => (
          <label key={i} className="row">
            <input type="checkbox" checked={chosen.some((c) => same(c, o.value))} onChange={(e) => onChange(e.target.checked ? [...chosen, o.value] : chosen.filter((c) => !same(c, o.value)))} />
            {isolate(o.label)}
          </label>
        ))}
      </div>
    );
  }
  const shown = value != null && value !== "" && !has(value) ? [{ value, label: String(value) }, ...options] : options;
  return (
    <div className="stack">
      {fetched && <input className="input" type="search" aria-label={t("Search")} placeholder={t("Search")} value={search} onChange={(e) => setSearch(e.target.value)} />}
      <select id={id} className="select" value={value == null ? "" : JSON.stringify(value)} onChange={(e) => onChange(e.target.value === "" ? null : JSON.parse(e.target.value))}>
        <option value="">{fetched && q.isPending ? t("Loading…") : t("Choose")}</option>
        {shown.map((o, i) => (
          <option key={i} value={JSON.stringify(o.value)}>
            {isolate(o.label)}
          </option>
        ))}
      </select>
      {fetched && q.isError && <p className="field-error">{(q.error as Error).message}</p>}
    </div>
  );
}

export function SettingField({ account, s, value, answers, onChange, error }: { account: string; s: Setting; value: unknown; answers: Record<string, unknown>; onChange: (v: unknown) => void; error?: string }) {
  const id = `set-${account}-${s.key}`;
  const shown = value ?? s.default ?? (s.type === "bool" ? false : "");
  let control;
  switch (s.type) {
    case "long":
      control = <textarea id={id} className="textarea" rows={3} value={String(shown)} onChange={(e) => onChange(e.target.value)} />;
      break;
    case "number":
      control = <input id={id} className="input" type="number" min={s.min ?? undefined} max={s.max ?? undefined} value={shown === "" ? "" : String(shown)} onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))} />;
      break;
    case "date":
      control = <input id={id} className="input" type="date" value={String(shown).slice(0, 10)} onChange={(e) => onChange(e.target.value || null)} />;
      break;
    case "bool":
      control = (
        <label className="row">
          <input id={id} type="checkbox" checked={shown === true} onChange={(e) => onChange(e.target.checked)} />
          {t("Yes")}
        </label>
      );
      break;
    case "select":
    case "multi":
      control = <Choose account={account} s={s} value={value ?? s.default ?? null} answers={answers} onChange={onChange} id={id} />;
      break;
    default:
      control = <input id={id} className="input" value={String(shown)} onChange={(e) => onChange(e.target.value)} />;
  }
  // A select or a list is a group of controls: its label names the group, not a single input.
  const group = s.type === "multi";
  return group ? (
    <div className={`field ${error ? "field-invalid" : ""}`}>
      <span id={`${id}-label`} className="field-label">
        {isolate(s.label)}
        {s.required && <span className="req" aria-hidden="true">*</span>}
      </span>
      {control}
      {error ? <p className="field-error">{error}</p> : s.help ? <p className="field-hint">{isolate(s.help)}</p> : null}
    </div>
  ) : (
    <Field label={isolate(s.label)} htmlFor={id} required={s.required} error={error} hint={s.help ? isolate(s.help) : undefined}>
      {control}
    </Field>
  );
}

/** The kind chooser and the settings that apply, for one account. */
export function AccountOptions({ account, rules, kind, answers, onKind, onAnswers, locked }: { account: string; rules: Rules; kind: string; answers: Record<string, unknown>; onKind: (k: string) => void; onAnswers: (a: Record<string, unknown>) => void; locked?: boolean }) {
  const current = kind || rules.kinds[0]?.name || null;
  const shown = asked(rules, current, answers);
  return (
    <fieldset className="stack" disabled={locked}>
      {rules.kinds.length > 1 && (
        <Field label={t("Kind of post")} htmlFor={`kind-${account}`}>
          <select id={`kind-${account}`} className="select" value={current ?? ""} onChange={(e) => onKind(e.target.value)}>
            {rules.kinds.map((k) => (
              <option key={k.name} value={k.name}>
                {isolate(kindLabel(k))}
              </option>
            ))}
          </select>
        </Field>
      )}
      {shown.map((s) => (
        <SettingField key={s.key} account={account} s={s} value={answers[s.key]} answers={answers} onChange={(v) => onAnswers({ ...answers, [s.key]: v })} />
      ))}
    </fieldset>
  );
}
