CREATE TABLE IF NOT EXISTS "users" (
  "id"          SERIAL PRIMARY KEY,
  "name"        TEXT                NOT NULL,
  "user_id"     INTEGER UNIQUE      NOT NULL,
  "is_admin"    BOOLEAN             NOT NULL,
  "is_banned"   BOOLEAN             NOT NULL
);