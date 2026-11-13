// The media library page (/media).

import { t } from "@ferrum/client";
import { PageHead, PageStack } from "@ferrum/kit";
import { MediaTabs } from "./media/Library";

export function MediaRoute() {
  return (
    <PageStack>
      <PageHead title={t("Media")} description={t("Pictures, video, GIFs and documents for posts, with their descriptions and credits.")} />
      <MediaTabs />
    </PageStack>
  );
}
