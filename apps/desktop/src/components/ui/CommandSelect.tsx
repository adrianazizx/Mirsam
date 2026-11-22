import { ChevronDown } from "lucide-react";
import {
  Button,
  type Key,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { type CommandId, isCommandEnabled, runCommand } from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import styles from "./CommandSelect.module.css";

export interface CommandOption {
  command: CommandId;
  /** Text in the closed control; the menu always shows the command's title. */
  shortLabel?: string;
}

export interface CommandSelectProps {
  /** Accessible name of the control. */
  label: string;
  /** The option matching the current state. */
  value: CommandId;
  options: CommandOption[];
  testId?: string;
}

/** A dropdown whose options are registry commands: choosing one runs it. */
export function CommandSelect({ label, value, options, testId }: CommandSelectProps) {
  const { t } = useTranslation();
  const enabled = useAppStore((state) => isCommandEnabled(value, state));
  const current = options.find((option) => option.command === value);
  const title = (command: CommandId) => t(`commands.${command}`);

  return (
    <Select
      aria-label={label}
      value={value}
      onChange={(key: Key | null) => {
        const option = options.find((candidate) => candidate.command === key);
        if (option) void runCommand(option.command);
      }}
      isDisabled={!enabled}
      className={styles.select}
      data-testid={testId}
    >
      <Button className={styles.button}>
        <SelectValue className={styles.value}>
          {() => current?.shortLabel ?? title(value)}
        </SelectValue>
        <ChevronDown className={styles.chevron} aria-hidden="true" />
      </Button>
      <Popover placement="top" className={styles.popover}>
        <ListBox className={styles.list}>
          {options.map((option) => (
            <ListBoxItem
              key={option.command}
              id={option.command}
              textValue={title(option.command)}
              className={styles.item}
            >
              {title(option.command)}
            </ListBoxItem>
          ))}
        </ListBox>
      </Popover>
    </Select>
  );
}
