-- Attribution for PersonFilterBar-style views: household (all rows) vs mine (owner_user_id = session user).
-- Legacy rows keep owner_user_id NULL (visible only in household aggregate lists).

ALTER TABLE assets
ADD COLUMN owner_user_id UUID REFERENCES users (id) ON DELETE SET NULL;

CREATE INDEX assets_installation_owner_idx ON assets (installation_id, owner_user_id);

ALTER TABLE liabilities
ADD COLUMN owner_user_id UUID REFERENCES users (id) ON DELETE SET NULL;

CREATE INDEX liabilities_installation_owner_idx ON liabilities (installation_id, owner_user_id);

ALTER TABLE budget_entries
ADD COLUMN owner_user_id UUID REFERENCES users (id) ON DELETE SET NULL;

CREATE INDEX budget_entries_installation_owner_idx ON budget_entries (installation_id, owner_user_id);

ALTER TABLE planning_flows
ADD COLUMN owner_user_id UUID REFERENCES users (id) ON DELETE SET NULL;

CREATE INDEX planning_flows_installation_owner_idx ON planning_flows (installation_id, owner_user_id);
