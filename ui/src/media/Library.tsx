// The media library (/media) and the picker the composer opens: the assets in a grid (kinds, search, tags, brand), several files
// uploaded at once, description and credit edited, an asset deleted (the server's refusal shown when it is used), stock pictures
// and GIFs found through the sources Social Settings name (the item's credit shown, "take" stores it), and a picture added from an
// address. One component serves both: with `onPick` the assets can be chosen and the tabs are the picker's.
// Calls: SocialMediaAsset list/create/PATCH/DELETE, POST /api/files, SocialBrand actions media_search / media_take /
// media_from_address, SocialSettings list.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { Link } from "react-router";
import { api, docPath, formatDecimal, isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Dialog, Field, ListSkeleton, StatePanel, Tabs, TabPanel, toast, useDebounced } from "@ferrum/kit";
import { change, make, record, remove, type Row, rows, run, text } from "../data";
import { factsOf } from "../composer/media";
import { Await, useAllowed, useBrands } from "../ui";

/** One stock result: what `media_search` answers for an item. */
export interface Found {
  id: string;
  preview: string | null;
  url: string | null;
  credit: string | null;
  credit_link: string | null;
  licence: string | null;
  width: number | null;
  height: number | null;
  alt: string | null;
  kind: string;
}

export const KINDS = ["image", "video", "document", "file"] as const;

function kindWord(kind: string): string {
  switch (kind) {
    case "image":
      return t("Pictures");
    case "video":
      return t("Video");
    case "document":
      return t("Documents");
    default:
      return t("Other files");
  }
}

/** The sources the settings name: connection names, one a line (the first is the default). */
export const sourcesOf = (settings: Row | undefined): string[] =>
  text(settings?.media_sources).split(/[,\n]/).map((s) => s.trim()).filter(Boolean);

const useSettings = () => useQuery({ queryKey: ["social", "settings"], queryFn: () => rows("SocialSettings").then((r) => r[0]).catch(() => undefined as Row | undefined), staleTime: 60_000 });

export function AssetThumb({ a }: { a: Row }) {
  return text(a.kind) === "image" ? <img className="media-thumb" src={text(a.file)} alt={text(a.alt)} loading="lazy" /> : <span className="media-thumb thumb-kind">{isolate(text(a.kind) || t("file"))}</span>;
}

function Edit({ a, onClose }: { a: Row; onClose: () => void }) {
  const client = useQueryClient();
  const [v, setV] = useState({ asset_name: text(a.asset_name), alt: text(a.alt), credit: text(a.credit), credit_link: text(a.credit_link), licence: text(a.licence) });
  const [refused, setRefused] = useState("");
  const save = useMutation({
    mutationFn: () => change("SocialMediaAsset", a.id, v, a.version),
    onSuccess: async () => { await client.invalidateQueries({ queryKey: ["social", "assets"] }); onClose(); },
    onError: (e) => setRefused(refusal(e)),
  });
  const set = (k: keyof typeof v) => (e: { target: { value: string } }) => setV({ ...v, [k]: e.target.value });
  return (
    <Dialog open onClose={onClose} title={t("Edit {name}", { name: isolate(text(a.asset_name)) })} footer={<><Button onClick={onClose}>{t("Cancel")}</Button><Button variant="primary" loading={save.isPending} onClick={() => save.mutate()}>{t("Save")}</Button></>}>
      <div className="stack">
        {refused && <Alert tone="error">{isolate(refused)}</Alert>}
        <Field label={t("Name")} htmlFor="as-name"><input id="as-name" className="input" value={v.asset_name} onChange={set("asset_name")} /></Field>
        <Field label={t("Description")} htmlFor="as-alt" hint={t("What it shows, for people who cannot see it.")}><input id="as-alt" className="input" value={v.alt} onChange={set("alt")} /></Field>
        <Field label={t("Credit")} htmlFor="as-credit"><input id="as-credit" className="input" value={v.credit} onChange={set("credit")} /></Field>
        <Field label={t("Credit link")} htmlFor="as-link"><input id="as-link" className="input" type="url" value={v.credit_link} onChange={set("credit_link")} /></Field>
        <Field label={t("Licence")} htmlFor="as-licence"><input id="as-licence" className="input" value={v.licence} onChange={set("licence")} /></Field>
      </div>
    </Dialog>
  );
}

/** Uploads several files into the library (each stored, then made an asset of the brand). */
export function Upload({ brand, onDone }: { brand: string; onDone: (assets: Row[]) => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");
  const go = async (files: FileList | null) => {
    if (!files?.length) return;
    setBusy(true);
    setRefused("");
    const made: Row[] = [];
    try {
      for (const f of Array.from(files)) {
        const form = new FormData();
        form.append("file", f);
        const [stored, facts] = await Promise.all([api.post<{ url: string }>("/api/files", form), factsOf(f)]);
        made.push(await make("SocialMediaAsset", { file: stored.url, brand: brand || null, asset_name: f.name, source: "Upload", ...facts }));
      }
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(false);
      if (made.length) onDone(made);
    }
  };
  return (
    <div className="stack">
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      <input ref={input} type="file" multiple hidden onChange={(e) => { void go(e.target.files); e.target.value = ""; }} />
      <div><Button icon="upload" loading={busy} onClick={() => input.current?.click()}>{t("Choose files to upload")}</Button></div>
    </div>
  );
}

function Address({ brand, onDone }: { brand: string; onDone: (assets: Row[]) => void }) {
  const [url, setUrl] = useState("");
  const [alt, setAlt] = useState("");
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");
  const go = async () => {
    setBusy(true);
    setRefused("");
    try {
      const out = await run<{ id: string }>("SocialBrand", brand, "media_from_address", { url, alt: alt || undefined });
      setUrl("");
      onDone([await record("SocialMediaAsset", out.id)]);
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="stack">
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      <Field label={t("Web address of the file")} htmlFor="ad-url" required><input id="ad-url" className="input" type="url" value={url} onChange={(e) => setUrl(e.target.value)} /></Field>
      <Field label={t("Description")} htmlFor="ad-alt"><input id="ad-alt" className="input" value={alt} onChange={(e) => setAlt(e.target.value)} /></Field>
      <div><Button variant="primary" loading={busy} disabled={!url.trim() || !brand} onClick={() => void go()}>{t("Fetch it")}</Button></div>
    </div>
  );
}

function Stock({ brand, onDone }: { brand: string; onDone: (assets: Row[]) => void }) {
  const settings = useSettings();
  const sources = sourcesOf(settings.data);
  const manager = useAllowed("SocialSettings", "write");
  const [source, setSource] = useState("");
  const [query, setQuery] = useState("");
  const asked = useDebounced(query.trim(), 400);
  const chosen = source || sources[0] || "";
  const [page, setPage] = useState(1);
  const found = useQuery({
    queryKey: ["social", "stock", brand, chosen, asked, page],
    queryFn: () => run<{ items: Found[]; source: string }>("SocialBrand", brand, "media_search", { query: asked, source: chosen || undefined, page }),
    enabled: !!brand && !!asked && sources.length > 0,
    retry: false,
  });
  const [busy, setBusy] = useState<string | null>(null);
  const [refused, setRefused] = useState("");
  const take = async (f: Found) => {
    setBusy(f.id);
    setRefused("");
    try {
      const out = await run<{ id: string }>("SocialBrand", brand, "media_take", { id: f.id, source: chosen || undefined, url: f.url ?? undefined, alt: f.alt ?? undefined, credit: f.credit ?? undefined, credit_link: f.credit_link ?? undefined, licence: f.licence ?? undefined, width: f.width ?? undefined, height: f.height ?? undefined });
      onDone([await record("SocialMediaAsset", out.id)]);
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  if (settings.isSuccess && sources.length === 0)
    return (
      <StatePanel kind="empty" title={t("No picture source is set")}>
        {t("An administrator names the sources of stock pictures and GIFs in Social Settings.")} {manager && <Link to="/apps/social/settings">{t("Open the settings")}</Link>}
      </StatePanel>
    );
  return (
    <div className="stack">
      <div className="row">
        {sources.length > 1 && (
          <Field label={t("Source")} htmlFor="st-source">
            <select id="st-source" className="select" value={chosen} onChange={(e) => { setSource(e.target.value); setPage(1); }}>
              {sources.map((s) => <option key={s} value={s}>{isolate(s)}</option>)}
            </select>
          </Field>
        )}
        <Field label={t("Look for")} htmlFor="st-q"><input id="st-q" className="input" type="search" value={query} onChange={(e) => { setQuery(e.target.value); setPage(1); }} /></Field>
      </div>
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      {found.isError && <Alert tone="error">{isolate(refusal(found.error))}</Alert>}
      {found.isFetching && <p className="muted">{t("Looking…")}</p>}
      {found.data && (found.data.items.length === 0 ? <p className="muted">{t("Nothing found.")}</p> : (
        <ul className="media-grid" aria-label={t("Results")}>
          {found.data.items.map((f) => (
            <li key={f.id} className="card card-body stack">
              {f.preview ? <img className="media-thumb" src={f.preview} alt={f.alt ?? ""} loading="lazy" /> : <span className="media-thumb thumb-kind">{isolate(f.kind)}</span>}
              {(f.credit || f.licence) && <span className="muted">{f.credit_link ? <a href={f.credit_link} target="_blank" rel="noreferrer noopener">{isolate(f.credit ?? "")}</a> : isolate(f.credit ?? "")} {f.licence && `· ${isolate(f.licence)}`}</span>}
              <Button size="sm" loading={busy === f.id} onClick={() => void take(f)}>{t("Take it")}</Button>
            </li>
          ))}
        </ul>
      ))}
      {found.data && found.data.items.length > 0 && (
        <div className="row"><Button size="sm" disabled={page < 2} onClick={() => setPage(page - 1)}>{t("Earlier results")}</Button><Button size="sm" onClick={() => setPage(page + 1)}>{t("More results")}</Button></div>
      )}
    </div>
  );
}

function Grid({ brand, onPick, picked }: { brand: string; onPick?: (a: Row) => void; picked: string[] }) {
  const client = useQueryClient();
  const canWrite = useAllowed("SocialMediaAsset", "write");
  const canDelete = useAllowed("SocialMediaAsset", "delete");
  const [q, setQ] = useState("");
  const [kind, setKind] = useState("");
  const [tag, setTag] = useState("");
  const [which, setWhich] = useState(brand);
  const brands = useBrands();
  const search = useDebounced(q.trim(), 300);
  const tags = useQuery({ queryKey: ["social", "asset-tags"], queryFn: () => api.get<{ tags: (string | { tag: string })[] }>("/api/tags/SocialMediaAsset").then((r) => r.tags.map((x) => (typeof x === "string" ? x : x.tag))).catch(() => [] as string[]) });
  const list = useQuery({
    queryKey: ["social", "assets", which, kind, tag, search],
    queryFn: () => api.get<{ rows: Row[] }>(docPath("SocialMediaAsset"), { filters: JSON.stringify([...(which ? [["brand", "=", which]] : []), ...(kind ? [["kind", "=", kind]] : []), ...(tag ? [["_tag", "=", tag]] : [])]), search: search || undefined, limit: 120, sort: "created_at desc" }).then((r) => r.rows),
  });
  const [editing, setEditing] = useState<Row | null>(null);
  const [refused, setRefused] = useState("");
  const drop = async (a: Row) => {
    if (!(await confirmDialog({ title: t("Delete {name}?", { name: isolate(text(a.asset_name)) }), message: t("It leaves the library for good."), confirmLabel: t("Delete"), danger: true }))) return;
    setRefused("");
    try {
      await remove("SocialMediaAsset", a.id);
      await client.invalidateQueries({ queryKey: ["social", "assets"] });
    } catch (e) {
      setRefused(refusal(e));
    }
  };
  return (
    <div className="stack">
      <div className="row">
        <Field label={t("Search")} htmlFor="lib-q"><input id="lib-q" className="input" type="search" value={q} onChange={(e) => setQ(e.target.value)} /></Field>
        <Field label={t("Kind")} htmlFor="lib-kind"><select id="lib-kind" className="select" value={kind} onChange={(e) => setKind(e.target.value)}><option value="">{t("All kinds")}</option>{KINDS.map((k) => <option key={k} value={k}>{kindWord(k)}</option>)}</select></Field>
        <Field label={t("Brand")} htmlFor="lib-brand"><select id="lib-brand" className="select" value={which} onChange={(e) => setWhich(e.target.value)}><option value="">{t("All brands")}</option>{(brands.data ?? []).map((b) => <option key={b.id} value={b.id}>{isolate(text(b.brand_name))}</option>)}</select></Field>
        <Field label={t("Tag")} htmlFor="lib-tag"><select id="lib-tag" className="select" value={tag} onChange={(e) => setTag(e.target.value)}><option value="">{t("Any tag")}</option>{(tags.data ?? []).map((g) => <option key={g} value={g}>{isolate(g)}</option>)}</select></Field>
      </div>
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      <Await q={list} skeleton={<ListSkeleton rows={4} />}>
        {(assets) =>
          assets.length === 0 ? (
            <StatePanel kind={q || kind || tag ? "no-matches" : "empty"} title={q || kind || tag ? undefined : t("The library is empty")} onClear={q || kind || tag ? () => { setQ(""); setKind(""); setTag(""); } : undefined}>{t("Upload files, find stock pictures, or add one from an address.")}</StatePanel>
          ) : (
            <ul className="media-grid" aria-label={t("Media library")}>
              {assets.map((a) => (
                <li key={a.id} className="card card-body stack" data-picked={picked.includes(a.id) || undefined}>
                  <AssetThumb a={a} />
                  <strong className="media-name">{isolate(text(a.asset_name))}</strong>
                  <span className="muted">{kindWord(text(a.kind))}{a.bytes != null && ` · ${formatDecimal(Number(a.bytes) / 1024, 0)} KB`}{text(a.source) && ` · ${text(a.source)}`}</span>
                  {text(a.credit) && <span className="muted">{isolate(text(a.credit))}</span>}
                  {!text(a.alt) && text(a.kind) === "image" && <Badge tone="amber">{t("No description")}</Badge>}
                  <div className="row">
                    {onPick && <Button size="sm" variant={picked.includes(a.id) ? "default" : "primary"} onClick={() => onPick(a)}>{picked.includes(a.id) ? t("Chosen") : t("Use")}</Button>}
                    {canWrite && <Button size="sm" onClick={() => setEditing(a)}>{t("Edit")}</Button>}
                    {canDelete && <Button size="sm" variant="danger" icon="trash" aria-label={t("Delete {name}", { name: isolate(text(a.asset_name)) })} onClick={() => void drop(a)} />}
                  </div>
                </li>
              ))}
            </ul>
          )
        }
      </Await>
      {editing && <Edit a={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

const TABS = () => [
  { id: "library", label: t("Library") },
  { id: "upload", label: t("Upload") },
  { id: "stock", label: t("Stock and GIFs") },
  { id: "address", label: t("From an address") },
];

/** The four ways to a medium. With `onPick` the assets are chosen (the composer's picker); without, they are added to the library. */
export function MediaTabs({ onPick, brandId }: { onPick?: (assets: Row[]) => void; brandId?: string }) {
  const client = useQueryClient();
  const brands = useBrands();
  const [tab, setTab] = useState("library");
  const [brand, setBrand] = useState(brandId ?? "");
  const [picked, setPicked] = useState<string[]>([]);
  const target = brand || brandId || brands.data?.[0]?.id || "";
  const added = async (assets: Row[]) => {
    await client.invalidateQueries({ queryKey: ["social", "assets"] });
    if (onPick) {
      setPicked((p) => [...p, ...assets.map((a) => a.id)]);
      onPick(assets);
    } else {
      toast(t("{n} added to the library", { n: String(assets.length) }));
      setTab("library");
    }
  };
  return (
    <div className="stack">
      {!brandId && tab !== "library" && (
        <Field label={t("For the brand")} htmlFor="media-for">
          <select id="media-for" className="select" value={target} onChange={(e) => setBrand(e.target.value)}>{(brands.data ?? []).map((b) => <option key={b.id} value={b.id}>{isolate(text(b.brand_name))}</option>)}</select>
        </Field>
      )}
      <Tabs id="media" tabs={TABS()} value={tab} onChange={setTab} />
      <TabPanel id="media" tab={tab}>
        {tab === "library" && <Grid brand={brandId ?? ""} picked={picked} onPick={onPick ? (a) => { setPicked((p) => [...p, a.id]); onPick([a]); } : undefined} />}
        {tab === "upload" && <Upload brand={target} onDone={added} />}
        {tab === "stock" && <Stock brand={target} onDone={added} />}
        {tab === "address" && <Address brand={target} onDone={added} />}
      </TabPanel>
    </div>
  );
}

export function MediaPicker({ onPick, onClose, brandId }: { onPick: (assets: Row[]) => void; onClose: () => void; brandId?: string }) {
  return (
    <Dialog open onClose={onClose} wide title={t("Add media")} footer={<Button variant="primary" onClick={onClose}>{t("Done")}</Button>}>
      <MediaTabs onPick={onPick} brandId={brandId} />
    </Dialog>
  );
}
