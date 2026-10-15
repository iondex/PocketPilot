CREATE TABLE discord_categories (
    bot TEXT NOT NULL, guild TEXT NOT NULL, workspace TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('dispatching','accepted','outcome_unknown')),
    channel TEXT,
    PRIMARY KEY(bot,guild,workspace)
);
CREATE TABLE discord_provisions (
    bot TEXT NOT NULL, guild TEXT NOT NULL, instance TEXT NOT NULL, thread TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('dispatching','accepted','outcome_unknown')),
    channel TEXT,
    PRIMARY KEY(bot,guild,instance,thread)
);
CREATE TABLE discord_bindings (
    bot TEXT NOT NULL, guild TEXT NOT NULL, channel TEXT NOT NULL,
    owner TEXT NOT NULL, workspace TEXT NOT NULL, instance TEXT NOT NULL,
    thread TEXT NOT NULL, cwd TEXT NOT NULL, endpoint TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1, revision INTEGER NOT NULL DEFAULT 1,
    observed_items TEXT NOT NULL DEFAULT '{}', history_ready INTEGER NOT NULL DEFAULT 0,
    card_message TEXT, card_text TEXT,
    PRIMARY KEY(bot,guild,channel), UNIQUE(bot,guild,instance,thread)
);
CREATE TRIGGER discord_binding_identity_immutable
BEFORE UPDATE OF instance,thread,workspace,cwd,endpoint,owner ON discord_bindings
BEGIN SELECT RAISE(ABORT,'Discord container identity is immutable'); END;
CREATE TABLE discord_deliveries (
    id TEXT PRIMARY KEY, bot TEXT NOT NULL, guild TEXT NOT NULL, channel TEXT NOT NULL,
    source_key TEXT NOT NULL, revision INTEGER NOT NULL, payload TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','dispatching','delivered','outcome_unknown','rejected')),
    not_before INTEGER NOT NULL DEFAULT 0,
    UNIQUE(bot,guild,channel,source_key)
);
