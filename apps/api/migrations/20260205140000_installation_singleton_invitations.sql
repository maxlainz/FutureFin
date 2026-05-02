-- One logical household per installation: if multiple rows exist (legacy dev), keep oldest.
DO $$
DECLARE
    keeper_id UUID;
    cnt INTEGER;
BEGIN
    SELECT
        COUNT(*) INTO cnt
    FROM
        households;
    IF cnt > 1 THEN
        SELECT
            id INTO keeper_id
        FROM
            households
        ORDER BY
            created_at ASC
        LIMIT 1;
        DELETE FROM households
        WHERE
            id <> keeper_id;
    END IF;
END
$$;

CREATE UNIQUE INDEX households_installation_singleton ON households ((1));

UPDATE households
SET
    name = 'FutureFin'
WHERE
    name IS DISTINCT FROM 'FutureFin';

CREATE TABLE household_invitations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    household_id UUID NOT NULL REFERENCES households (id) ON DELETE CASCADE,
    invitee_username TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('member', 'viewer')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'cancelled')),
    created_by UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now (),
    resolved_at TIMESTAMPTZ,
    CONSTRAINT household_invitations_username_len CHECK (
        char_length(invitee_username) BETWEEN 3 AND 64
    )
);

CREATE UNIQUE INDEX household_invitations_one_pending_per_username ON household_invitations (household_id, invitee_username)
WHERE
    status = 'pending';

CREATE INDEX household_invitations_household_status_idx ON household_invitations (household_id, status);
