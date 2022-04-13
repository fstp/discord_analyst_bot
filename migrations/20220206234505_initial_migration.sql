CREATE TABLE IF NOT EXISTS "Users" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "name"        TEXT                NOT NULL,
  "is_admin"    BOOLEAN             NOT NULL DEFAULT false,
  "is_banned"   BOOLEAN             NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS "Guilds" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "name"        TEXT                NOT NULL,
  "is_banned"   BOOLEAN             NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS "Channels" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "name"        TEXT                NOT NULL,
  "guild"       INTEGER             NOT NULL,
  "webhook"     INTEGER UNIQUE      NOT NULL,
  FOREIGN KEY ("guild") REFERENCES "Guilds"("id") ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS "Connections" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "source"      INTEGER             NOT NULL,
  "target"      INTEGER             NOT NULL,
  "user"        INTEGER             NOT NULL,
  "webhook"     INTEGER             NOT NULL,
  FOREIGN KEY ("source")  REFERENCES "Channels"("id") ON DELETE CASCADE,
  FOREIGN KEY ("target")  REFERENCES "Channels"("id") ON DELETE CASCADE
  UNIQUE (source, target, user, webhook)
);

CREATE TABLE IF NOT EXISTS "Mentions" (
  "id"          INTEGER PRIMARY KEY NOT NULL,
  "source"      INTEGER,
  "target"      INTEGER             NOT NULL,
  "mention"     TEXT                NOT NULL,
  "user"        INTEGER             NOT NULL,
  FOREIGN KEY ("source")  REFERENCES "Channels"("id") ON DELETE CASCADE,
  FOREIGN KEY ("target")  REFERENCES "Channels"("id") ON DELETE CASCADE
);