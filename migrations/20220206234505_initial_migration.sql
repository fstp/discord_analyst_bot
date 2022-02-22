CREATE TABLE IF NOT EXISTS "Users" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "name"        TEXT                NOT NULL,
  "is_admin"    BOOLEAN             NOT NULL,
  "is_banned"   BOOLEAN             NOT NULL
);

CREATE TABLE IF NOT EXISTS "Guilds" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "name"        TEXT                NOT NULL
);

CREATE TABLE IF NOT EXISTS "Channels" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "name"        TEXT                NOT NULL,
  "guild_id"    INTEGER             NOT NULL,
  FOREIGN KEY ("guild_id") REFERENCES "Guilds"("id")
);

CREATE TABLE IF NOT EXISTS "Webhooks" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "user_id"     INTEGER             NOT NULL,
  "channel_id"  INTEGER             NOT NULL,
  FOREIGN KEY ("channel_id") REFERENCES "Channels"("id")
);

CREATE TABLE IF NOT EXISTS "Connections" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "source"      INTEGER             NOT NULL,
  "target"      INTEGER             NOT NULL,
  "webhook"     INTEGER             NOT NULL,
  "user"        INTEGER             NOT NULL,
  FOREIGN KEY ("source")  REFERENCES "Channels"("id"),
  FOREIGN KEY ("target")  REFERENCES "Channels"("id"),
  FOREIGN KEY ("webhook") REFERENCES "Webhooks"("id")
);