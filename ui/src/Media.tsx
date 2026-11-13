// The media library page (/media).

import { t } from "@ferrum/client";
import { PageHead, PageStack } from "@ferrum/kit";
import { MediaTabs } from "./media/Library";
import { Nav } from "./ui";

export function MediaRoute() {
  return (
    <PageStack>
      <Nav />
      <PageHead title={t("Media")} description={t("Pictures, video, GIFs and documents for posts, with their descriptions and credits.")} />
      <MediaTabs />
    </PageStack>
  );
}
