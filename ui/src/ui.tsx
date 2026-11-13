// What the screens share: the way between them, the person's rights, the states a request goes through, and the lists
// of brands and accounts every screen starts from.

import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useLocation } from "react-router";
import { initials, isolate, t } from "@ferrum/client";
import { ButtonLink, FormSkeleton, LoadFailed } from "@ferrum/kit";
import { can, useMetaIndex, type PermKeyLike } from "./rights";
import { type Row, rows, text } from "./data";

export { can, type PermKeyLike };

/** The screens of the app, one press apart on every page. */
export function Nav() {
  const here = useLocation().pathname;
  const items: [string, string, string][] = [
    ["/", t("Home"), "home"],
    ["/calendar", t("Calendar"), "calendar"],
    ["/queue", t("Posting times"), "timeline"],
    ["/accounts", t("Accounts"), "apps"],
    ["/media", t("Media"), "file"],
    ["/inbox", t("Inbox"), "mail"],
    ["/numbers", t("Numbers"), "chart"],
    ["/library", t("Library"), "doc"],
    ["/automation", t("Feeds and plugs"), "refresh"],
  ];
  return (
    <nav className="row" aria-label={t("Social")}>
      {items.map(([to, label, icon]) => (
        <ButtonLink key={to} to={to} size="sm" icon={icon} variant={(to === "/" ? here === "/" : here.startsWith(to)) ? "default" : "ghost"}>
          {label}
        </ButtonLink>
      ))}
    </nav>
  );
}

/** Whether the person may do `key` to `entity`: true until the metadata is in (the server decides; this keeps buttons it would refuse from being offered). */
export function useAllowed(entity: string, key: PermKeyLike): boolean {
  const perms = useMetaIndex().data?.entities.find((e) => e.name === entity)?.permissions;
  return !perms || can(perms, key);
}

/** A request's three states told apart: waiting (a skeleton the shape of the content), refused or failed (with its words and a way to try again), and the data. */
export function Await<T>({ q, skeleton, children }: { q: UseQueryResult<T>; skeleton?: ReactNode; children: (data: T) => ReactNode }) {
  if (q.isPending) return <>{skeleton ?? <FormSkeleton />}</>;
  if (q.isError) return <LoadFailed error={q.error} onRetry={() => void q.refetch()} />;
  return <>{children(q.data)}</>;
}

/** Of several requests the one that says what the page can be: the first that failed, else the first still waiting, else the first. */
export function firstOf(...qs: UseQueryResult<unknown>[]): UseQueryResult<unknown> {
  return qs.find((q) => q.isError) ?? qs.find((q) => q.isPending) ?? qs[0];
}

/** An account's or a brand's picture, else its initials. */
export function Face({ name, picture, small }: { name: string; picture?: string | null; small?: boolean }) {
  const size = small ? "face face-sm" : "face";
  return picture ? <img className={size} src={picture} alt="" loading="lazy" /> : <span className={`${size} face-initial`} aria-hidden="true">{initials(name)}</span>;
}

export const useBrands = () => useQuery({ queryKey: ["social", "brands"], queryFn: () => rows("SocialBrand", [], { sort: "brand_name asc" }) });

/** Accounts in the order the composer lists them: the account's own order, then its name. */
export const byOrder = (a: Row, b: Row) => (Number(a.sort_order ?? 0) - Number(b.sort_order ?? 0)) || text(a.account_name).localeCompare(text(b.account_name));
export const useAccounts = () => useQuery({ queryKey: ["social", "accounts"], queryFn: () => rows("SocialAccount").then((r) => [...r].sort(byOrder)) });

export const brandName = (brands: Row[] | undefined, id: unknown) => text(brands?.find((b) => b.id === id)?.brand_name);
export const nameOf = (r: Row) => isolate(text(r.display_name) || text(r.account_name));
