-- Civil calendar for derive-principal and UI parity with Mac "start of day" in local TZ.

ALTER TABLE installation
ADD COLUMN calendar_tz TEXT NOT NULL DEFAULT 'UTC'
CHECK (
    char_length(calendar_tz) BETWEEN 3 AND 64
    AND char_length(trim(calendar_tz)) > 0
    AND trim(calendar_tz) = calendar_tz
);
