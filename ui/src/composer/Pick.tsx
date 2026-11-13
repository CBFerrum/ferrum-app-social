// The accounts a post goes to, grouped by brand (a post is one brand's: once an account is chosen the other brands'
// wait), with the sets of the brand to start from.

import { isolate, t } from "@ferrum/client";
import { Badge } from "@ferrum/kit";
import { type Row, STATE_TONES, stateWord, text } from "../data";
import { Face, nameOf } from "../ui";

export function Pick({ accounts, brands, chosen, onToggle, locked }: { accounts: Row[]; brands: Row[]; chosen: string[]; onToggle: (id: string) => void; locked?: boolean }) {
  const brand = text(accounts.find((a) => chosen.includes(a.id))?.brand);
  const groups = [...new Set(accounts.map((a) => text(a.brand)))];
  return (
    <fieldset className="stack" disabled={locked}>
      <legend className="field-label">
        {t("Accounts")}
        <span className="req" aria-hidden="true">*</span>
      </legend>
      {groups.map((b) => (
        <div key={b} role="group" aria-label={isolate(text(brands.find((x) => x.id === b)?.brand_name) || t("No brand"))} className="stack">
          <strong>{isolate(text(brands.find((x) => x.id === b)?.brand_name) || t("No brand"))}</strong>
          <div className="row">
            {accounts
              .filter((a) => text(a.brand) === b)
              .map((a) => {
                const off = a.active === false;
                const other = !!brand && brand !== b;
                const on = chosen.includes(a.id);
                return (
                  <label key={a.id} className="chip" title={off ? t("Paused") : other ? t("Another brand's") : undefined}>
                    <input type="checkbox" checked={on} disabled={(off || other) && !on} aria-label={`${text(a.display_name) || text(a.account_name)}${text(a.state) ? `, ${stateWord(text(a.state))}` : ""}${off ? `, ${t("Paused")}` : ""}`} onChange={() => onToggle(a.id)} />
                    <Face name={text(a.account_name)} picture={text(a.picture)} />
                    {nameOf(a)}
                    {text(a.state) && text(a.state) !== "Connected" && <Badge tone={STATE_TONES[text(a.state) as keyof typeof STATE_TONES] ?? "neutral"}>{stateWord(text(a.state))}</Badge>}
                  </label>
                );
              })}
          </div>
        </div>
      ))}
    </fieldset>
  );
}
