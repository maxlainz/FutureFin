-- Mac parity: optional "derive principal from recurring payment" flag (principal still stored as computed snapshot).

ALTER TABLE liabilities
ADD COLUMN principal_derived_from_plan BOOLEAN NOT NULL DEFAULT false;
