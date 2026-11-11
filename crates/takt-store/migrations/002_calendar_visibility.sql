-- takt's own show/hide choice for a calendar. NULL follows Google's `selected`.
-- Never written by a calendar-list sync page.
ALTER TABLE calendars ADD COLUMN local_visible INTEGER;
