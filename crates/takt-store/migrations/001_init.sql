CREATE TABLE accounts (
    account_id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'ok',
    calendar_list_sync_token TEXT,
    added_at TEXT NOT NULL
);

CREATE TABLE account_settings (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (account_id, key)
);

CREATE TABLE colors (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('calendar', 'event')),
    color_id TEXT NOT NULL,
    -- This colour's entry from colors.get, e.g. {"background": ..., "foreground": ...}.
    raw_json TEXT NOT NULL,
    background TEXT NOT NULL,
    foreground TEXT NOT NULL,
    PRIMARY KEY (account_id, kind, color_id)
);

CREATE TABLE calendars (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    calendar_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    summary TEXT NOT NULL,
    time_zone TEXT,
    background_color TEXT,
    foreground_color TEXT,
    access_role TEXT,
    selected INTEGER NOT NULL DEFAULT 0,
    hidden INTEGER NOT NULL DEFAULT 0,
    is_primary INTEGER NOT NULL DEFAULT 0,
    events_sync_token TEXT,
    PRIMARY KEY (account_id, calendar_id)
);

-- start_utc, end_utc, series_end_utc: unix seconds. All-day dates are stored as UTC midnight.
-- original_start: RFC 3339 UTC for timed series, YYYY-MM-DD for all-day.
CREATE TABLE events (
    account_id TEXT NOT NULL,
    calendar_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('single', 'master', 'exception')),
    status TEXT,
    ical_uid TEXT,
    recurring_event_id TEXT,
    original_start TEXT,
    start_utc INTEGER,
    end_utc INTEGER,
    all_day INTEGER NOT NULL DEFAULT 0,
    series_end_utc INTEGER,
    etag TEXT,
    updated TEXT,
    PRIMARY KEY (account_id, calendar_id, event_id),
    FOREIGN KEY (account_id, calendar_id) REFERENCES calendars (account_id, calendar_id) ON DELETE CASCADE
);

CREATE INDEX events_range ON events (kind, start_utc, end_utc);
CREATE INDEX events_series ON events (kind, start_utc, series_end_utc);
CREATE INDEX events_exceptions ON events (account_id, calendar_id, recurring_event_id);

CREATE TABLE task_lists (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    list_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    title TEXT NOT NULL,
    updated TEXT,
    updated_min_checkpoint TEXT,
    PRIMARY KEY (account_id, list_id)
);

CREATE TABLE tasks (
    account_id TEXT NOT NULL,
    list_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT,
    due_date TEXT,
    parent TEXT,
    position TEXT,
    completed TEXT,
    hidden INTEGER NOT NULL DEFAULT 0,
    updated TEXT,
    PRIMARY KEY (account_id, list_id, task_id),
    FOREIGN KEY (account_id, list_id) REFERENCES task_lists (account_id, list_id) ON DELETE CASCADE
);

CREATE INDEX tasks_due ON tasks (due_date);
