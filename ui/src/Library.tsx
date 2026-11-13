// The library (/library): templates, sets, snippets, signatures, variables, link templates and saved replies in one place, as
// tabs. A template or a set starts a post in the composer; the rest are edited in the records frame's own forms (generic forms
// are enough for them), and deleted here with a confirmation. Calls: the list of each entity, DELETE.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, ButtonLink, confirmDialog, ListSkeleton, PageHead, PageStack, StatePanel, TabPanel, Tabs } from "@ferrum/kit";
import { remove, type Row, rows, text } from "./data";
import { Await, brandName, useAccounts, useAllowed, useBrands } from "./ui";

interface Kind {
  id: string;
  label: () => string;
  entity: string;
  name: string;
  /** The columns after the name. */
  cells: (r: Row, ctx: { brands: Row[]; accounts: Row[] }) => string;
  /** A row's own marks (a default signature). */
  mark?: (r: Row) => string | null;
  /** The page that starts a post from a row. */
  use?: (r: Row) => string;
}

const scope = (r: Row, c: { brands: Row[]; accounts: Row[] }) =>
  text(r.account) ? text(c.accounts.find((a) => a.id === r.account)?.account_name) : text(r.brand) ? brandName(c.brands, r.brand) : t("The whole workspace");

export const KINDS: Kind[] = [
  { id: "templates", label: () => t("Templates"), entity: "SocialTemplate", name: "template_name", cells: (r, c) => (text(r.brand) ? brandName(c.brands, r.brand) : ""), use: (r) => `/compose?template=${r.id}` },
  { id: "sets", label: () => t("Sets"), entity: "SocialSet", name: "set_name", cells: (r, c) => brandName(c.brands, r.brand), use: (r) => `/compose?set=${r.id}` },
  { id: "snippets", label: () => t("Snippets"), entity: "SocialSnippet", name: "snippet_name", cells: (r) => text(r.kind) },
  { id: "signatures", label: () => t("Signatures"), entity: "SocialSignature", name: "signature_name", cells: scope, mark: (r) => (r.is_default === true ? t("Default") : null) },
  { id: "variables", label: () => t("Variables"), entity: "SocialVariable", name: "variable_name", cells: (r, c) => `${text(r.value)} · ${scope(r, c)}` },
  { id: "links", label: () => t("Link templates"), entity: "SocialLinkTemplate", name: "template_name", cells: scope, mark: (r) => (r.active === false ? t("Off") : null) },
  { id: "replies", label: () => t("Saved replies"), entity: "SocialSavedReply", name: "title", cells: (r) => text(r.text).slice(0, 80) },
];

function Tab({ k }: { k: Kind }) {
  const client = useQueryClient();
  const brands = useBrands();
  const accounts = useAccounts();
  const canCreate = useAllowed(k.entity, "create");
  const canWrite = useAllowed(k.entity, "write");
  const canDelete = useAllowed(k.entity, "delete");
  const list = useQuery({ queryKey: ["social", "library", k.entity], queryFn: () => rows(k.entity, [], { sort: `${k.name} asc` }) });
  const [refused, setRefused] = useState("");
  const drop = async (r: Row) => {
    if (!(await confirmDialog({ title: t("Delete {name}?", { name: isolate(text(r[k.name])) }), message: t("It is removed for good."), confirmLabel: t("Delete"), danger: true }))) return;
    setRefused("");
    try {
      await remove(k.entity, r.id);
      await client.invalidateQueries({ queryKey: ["social", "library"] });
    } catch (e) {
      setRefused(refusal(e));
    }
  };
  const ctx = { brands: brands.data ?? [], accounts: accounts.data ?? [] };
  return (
    <div className="stack">
      <div className="row">
        <span className="spacer" />
        {canCreate && <ButtonLink to={`/e/${k.entity}/new`} variant="primary" icon="plus">{t("New")}</ButtonLink>}
      </div>
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      <Await q={list} skeleton={<ListSkeleton rows={4} />}>
        {(items) =>
          items.length === 0 ? (
            <StatePanel kind="empty" title={t("Nothing here yet")}>{t("What you add shows here.")}</StatePanel>
          ) : (
            <table className="table">
              <thead><tr><th scope="col">{t("Name")}</th><th scope="col">{t("Details")}</th><th scope="col"><span className="sr-only">{t("Actions")}</span></th></tr></thead>
              <tbody>
                {items.map((r) => (
                  <tr key={r.id}>
                    <td>
                      <Link to={`/e/${k.entity}/${r.id}`}>{isolate(text(r[k.name]))}</Link> {k.mark?.(r) && <Badge tone="blue">{k.mark(r)}</Badge>}
                    </td>
                    <td className="muted">{isolate(k.cells(r, ctx))}</td>
                    <td className="num">
                      {k.use && <ButtonLink size="sm" to={k.use(r)}>{t("Use")}</ButtonLink>}{" "}
                      {canWrite && <ButtonLink size="sm" to={`/e/${k.entity}/${r.id}`}>{t("Edit")}</ButtonLink>}{" "}
                      {canDelete && <Button size="sm" variant="danger" icon="trash" aria-label={t("Delete {name}", { name: isolate(text(r[k.name])) })} onClick={() => void drop(r)} />}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        }
      </Await>
    </div>
  );
}

export function LibraryRoute() {
  const [tab, setTab] = useState("templates");
  const k = KINDS.find((x) => x.id === tab) ?? KINDS[0];
  return (
    <PageStack>
      <PageHead title={t("Library")} description={t("What posts are made from: templates and sets, snippets, signatures, variables, link templates and saved replies.")} />
      <Tabs id="library" tabs={KINDS.map((x) => ({ id: x.id, label: x.label() }))} value={tab} onChange={setTab} />
      <TabPanel id="library" tab={tab}>
        <Tab key={k.id} k={k} />
      </TabPanel>
    </PageStack>
  );
}
