import { ExternalLink, X } from "lucide-react";
import { Toolbar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { isOpenable } from "../../state/objects";
import { describeTarget } from "../edit/linkText";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import styles from "./LinkTip.module.css";

/**
 * Where the link under the pointer goes (ADR 0051). A pressed web link is
 * pinned here with its address, and opens only from this button: the spec's
 * "links open only after showing the target". Pages open at once.
 */
export function LinkTip({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const tip = useAppStore((state) =>
    state.linkTip?.documentId === documentId ? state.linkTip : null,
  );
  if (!tip) return null;
  const { target } = tip.link;
  const shown = <span className={styles.target}>{describeTarget(target, t, locale)}</span>;
  return (
    <div className={styles.band}>
      {tip.pinned ? (
        // Pressed: the address, and the one button that opens it.
        <Toolbar
          className={styles.tip}
          aria-label={t("links.tipLabel")}
          dir={directionOf(locale)}
          data-testid="link-tip"
        >
          {shown}
          <span className={styles.actions}>
            {isOpenable(target) ? (
              <Button
                variant="primary"
                icon={<ExternalLink />}
                onPress={() => void runCommand("link.open")}
              >
                {t("links.open")}
              </Button>
            ) : (
              <span className={styles.note}>{t("links.notOpened")}</span>
            )}
            <IconButton
              label={t("commands.link.dismiss")}
              onPress={() => void runCommand("link.dismiss")}
            >
              <X />
            </IconButton>
          </span>
        </Toolbar>
      ) : (
        <div className={styles.tip} role="tooltip" dir={directionOf(locale)} data-testid="link-tip">
          {shown}
        </div>
      )}
    </div>
  );
}
