import { useId, useRef, useState } from "react";

import {
  Button,
  Field,
  FormSection,
  SelectInput,
  TextInput,
} from "../components/Controls";
import { SettingsPage } from "../components/SettingsPage";
import { useSettings } from "../settings/SettingsContext";
import type { AccountDraft, SettingsDraft } from "../settings/types";
import "./AccountsPage.css";

export function AccountsPage() {
  const { draft, setDraft, options } = useSettings();
  const serverId = useId();
  const accountInputId = useId();
  const initialAccountCount = draft?.accounts.accounts.length ?? 0;
  const nextRowId = useRef(initialAccountCount);
  const [rowIds, setRowIds] = useState<number[]>(() =>
    Array.from({ length: initialAccountCount }, (_, index) => index),
  );
  const [visiblePasswordIds, setVisiblePasswordIds] = useState<Set<number>>(
    () => new Set(),
  );

  if (!draft || !options) return null;

  const updateAccounts = (
    update: (accounts: SettingsDraft["accounts"]) => SettingsDraft["accounts"],
  ) => {
    setDraft((current) =>
      current ? { ...current, accounts: update(current.accounts) } : current,
    );
  };

  const updateAccount = (index: number, update: Partial<AccountDraft>) => {
    updateAccounts((accounts) => ({
      ...accounts,
      accounts: accounts.accounts.map((account, accountIndex) =>
        accountIndex === index ? { ...account, ...update } : account,
      ),
    }));
  };

  const togglePassword = (rowId: number) => {
    setVisiblePasswordIds((current) => {
      const next = new Set(current);
      if (next.has(rowId)) next.delete(rowId);
      else next.add(rowId);
      return next;
    });
  };

  const removeAccount = (index: number, rowId: number) => {
    setRowIds((current) => current.filter((id) => id !== rowId));
    setVisiblePasswordIds((current) => {
      if (!current.has(rowId)) return current;
      const next = new Set(current);
      next.delete(rowId);
      return next;
    });
    updateAccounts((accounts) => ({
      ...accounts,
      accounts: accounts.accounts.filter(
        (_account, accountIndex) => accountIndex !== index,
      ),
    }));
  };

  const addAccount = () => {
    const rowId = nextRowId.current;
    nextRowId.current += 1;
    setRowIds((current) => [...current, rowId]);
    updateAccounts((accounts) => ({
      ...accounts,
      accounts: [...accounts.accounts, { username: "", password: "" }],
    }));
  };

  return (
    <SettingsPage
      title="Accounts"
      description="Choose an EverQuest server and manage the credentials Stonemite uses to sign in."
    >
      <FormSection
        title="Server"
        description="The EverQuest server used for these accounts."
      >
        <Field label="EverQuest server" htmlFor={serverId}>
          <SelectInput
            id={serverId}
            value={draft.accounts.server}
            options={options.servers}
            onChange={(event) =>
              updateAccounts((accounts) => ({
                ...accounts,
                server: event.currentTarget.value,
              }))
            }
          />
        </Field>
      </FormSection>

      <FormSection
        title="EverQuest accounts"
        description="Passwords are encrypted with Windows Data Protection (DPAPI). Only your Windows user account on this PC can decrypt them."
      >
        {draft.accounts.accounts.length === 0 ? (
          <p className="accounts-empty">No accounts have been added.</p>
        ) : (
          <ol className="account-list" aria-label="EverQuest accounts">
            {draft.accounts.accounts.map((account, index) => {
              const rowId = rowIds[index];
              if (rowId === undefined) return null;

              const usernameId = `${accountInputId}-username-${rowId}`;
              const passwordId = `${accountInputId}-password-${rowId}`;
              const passwordVisible = visiblePasswordIds.has(rowId);
              const accountNumber = index + 1;
              const autocompleteSection = `section-account${rowId}`;

              return (
                <li className="account-row" key={rowId}>
                  <fieldset>
                    <legend>Account {accountNumber}</legend>
                    <div className="account-field">
                      <label htmlFor={usernameId}>Username</label>
                      <TextInput
                        id={usernameId}
                        name={`account-${rowId}-username`}
                        type="text"
                        value={account.username}
                        autoComplete={`${autocompleteSection} username`}
                        autoCapitalize="none"
                        spellCheck={false}
                        onChange={(event) =>
                          updateAccount(index, {
                            username: event.currentTarget.value,
                          })
                        }
                      />
                    </div>
                    <div className="account-field">
                      <label htmlFor={passwordId}>Password</label>
                      <div className="account-password-control">
                        <TextInput
                          id={passwordId}
                          name={`account-${rowId}-password`}
                          type={passwordVisible ? "text" : "password"}
                          value={account.password}
                          autoComplete={`${autocompleteSection} current-password`}
                          spellCheck={false}
                          onChange={(event) =>
                            updateAccount(index, {
                              password: event.currentTarget.value,
                            })
                          }
                        />
                        <Button
                          type="button"
                          aria-controls={passwordId}
                          aria-label={`${passwordVisible ? "Hide" : "Show"} password for account ${accountNumber}`}
                          aria-pressed={passwordVisible}
                          onClick={() => togglePassword(rowId)}
                        >
                          {passwordVisible ? "Hide" : "Show"}
                        </Button>
                      </div>
                    </div>
                    <div className="account-actions">
                      <Button
                        type="button"
                        variant="quiet"
                        className="account-remove"
                        aria-label={`Remove account ${accountNumber}`}
                        onClick={() => removeAccount(index, rowId)}
                      >
                        Remove
                      </Button>
                    </div>
                  </fieldset>
                </li>
              );
            })}
          </ol>
        )}

        <div className="account-add-row">
          <Button type="button" onClick={addAccount}>
            Add account
          </Button>
        </div>
      </FormSection>
    </SettingsPage>
  );
}
