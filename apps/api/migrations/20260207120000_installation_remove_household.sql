-- Replace "household" storage names with installation-scoped tables (single row per deployment).

-- Remove display name — not part of the product model on the wire.
ALTER TABLE households DROP CONSTRAINT IF EXISTS households_name_len;

ALTER TABLE households DROP COLUMN name;

ALTER TABLE households RENAME TO installation;

ALTER TABLE installation RENAME CONSTRAINT households_pkey TO installation_pkey;

ALTER TABLE installation RENAME CONSTRAINT households_currency_fmt TO installation_currency_fmt;

ALTER TABLE installation RENAME CONSTRAINT households_target_age TO installation_target_age;

ALTER TABLE installation RENAME CONSTRAINT households_show_age_mode TO installation_show_age_mode;

ALTER INDEX households_installation_singleton RENAME TO installation_singleton;

ALTER TABLE household_memberships RENAME COLUMN household_id TO installation_id;

ALTER TABLE household_memberships RENAME TO installation_memberships;

ALTER INDEX household_memberships_user_id_idx RENAME TO installation_memberships_user_id_idx;

ALTER INDEX household_memberships_household_id_idx RENAME TO installation_memberships_installation_id_idx;

ALTER TABLE household_invitations RENAME COLUMN household_id TO installation_id;

ALTER TABLE household_invitations RENAME TO installation_invitations;

ALTER INDEX household_invitations_one_pending_per_username RENAME TO installation_invitations_one_pending_per_username;

ALTER INDEX household_invitations_household_status_idx RENAME TO installation_invitations_installation_status_idx;

ALTER TABLE persons RENAME COLUMN household_id TO installation_id;

ALTER INDEX persons_household_id_idx RENAME TO persons_installation_id_idx;

ALTER INDEX persons_one_primary_per_household RENAME TO persons_one_primary_per_installation;
