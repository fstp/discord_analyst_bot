CREATE TABLE IF NOT EXISTS "Users" (
  "id"          SERIAL PRIMARY KEY,
  "name"        TEXT               NOT NULL,
  "user_id"     INTEGER UNIQUE     NOT NULL,
  "is_admin"    BOOLEAN            NOT NULL,
  "is_banned"   BOOLEAN            NOT NULL
);

CREATE TABLE IF NOT EXISTS "Guilds" (
  "id"          PRIMARY KEY,
  "name"        TEXT        NOT NULL
);

CREATE TABLE IF NOT EXISTS "Channels" (
  "id"          PRIMARY KEY,
  "name"        TEXT        NOT NULL,
  "guild_id"    INTEGER     NOT NULL,
  FOREIGN KEY ("guild_id") REFERENCES "Guilds"("id")
);

CREATE TABLE IF NOT EXISTS "Webhooks" (
  "id"          PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS "Connections" (
  "id"          SERIAL PRIMARY KEY,
  "source"      INTEGER            NOT NULL,
  "target"      INTEGER            NOT NULL,
  "webhook"     INTEGER            NOT NULL,
  FOREIGN KEY ("source")  REFERENCES "Channels"("id"),
  FOREIGN KEY ("target")  REFERENCES "Channels"("id"),
  FOREIGN KEY ("webhook") REFERENCES "Webhooks"("id")
);