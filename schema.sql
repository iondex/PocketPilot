-- Prototype schema. Edit directly until stable 0.1; no migration runner.
-- IF NOT EXISTS preserves existing PocketPilot data when opening the ledger.

-- Rebuildable metadata and independent observation settings shared by integrations.
CREATE TABLE IF NOT EXISTS integration_session_cache (
    instance TEXT NOT NULL, thread TEXT NOT NULL, payload TEXT NOT NULL,
    checked_at INTEGER NOT NULL, PRIMARY KEY(instance,thread)
);
CREATE TABLE IF NOT EXISTS integration_binding_preferences (
    key TEXT PRIMARY KEY, paused INTEGER NOT NULL, revision INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS bindings (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    active INTEGER NOT NULL DEFAULT 1,
    workspace_id TEXT,
    display_name TEXT,
    status_message_id INTEGER,
    status_text TEXT,
    observed_items TEXT NOT NULL DEFAULT '{}',
    history_ready INTEGER NOT NULL DEFAULT 0,
    topic_title TEXT,
    PRIMARY KEY(bot_id,chat_id,topic_id)
);

CREATE TABLE IF NOT EXISTS callbacks (
    token TEXT PRIMARY KEY,
    bot_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    config_hash TEXT NOT NULL,
    payload TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS catalog_providers (
    id TEXT PRIMARY KEY,
    host_id TEXT NOT NULL,
    payload TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS catalog_workspaces (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES catalog_providers(id),
    payload TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_exclusions (
    provider_id TEXT NOT NULL REFERENCES catalog_providers(id),
    cwd TEXT NOT NULL,
    PRIMARY KEY(provider_id,cwd)
);

CREATE TABLE IF NOT EXISTS session_list_snapshots (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    paths TEXT NOT NULL,
    payload TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS session_errors (
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    observed_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY(instance_id,thread_id,turn_id,item_id)
);

CREATE TABLE IF NOT EXISTS containers (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    PRIMARY KEY(bot_id, chat_id, topic_id)
);

CREATE TABLE IF NOT EXISTS deliveries (
    id TEXT PRIMARY KEY,
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    source_key TEXT NOT NULL,
    payload TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending','dispatching','delivered','outcome_unknown','rejected')),
    message_id INTEGER,
    not_before INTEGER NOT NULL DEFAULT 0,
    batch_opened_ms INTEGER,
    batch_members INTEGER NOT NULL DEFAULT 0,
    next_part INTEGER NOT NULL DEFAULT 0,
    rendered_parts TEXT,
    UNIQUE (bot_id, chat_id, topic_id, source_key)
);

CREATE TABLE IF NOT EXISTS discord_bindings (
    bot TEXT NOT NULL,
    guild TEXT NOT NULL,
    channel TEXT NOT NULL,
    owner TEXT NOT NULL,
    workspace TEXT NOT NULL,
    instance TEXT NOT NULL,
    thread TEXT NOT NULL,
    cwd TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    revision INTEGER NOT NULL DEFAULT 1,
    observed_items TEXT NOT NULL DEFAULT '{}',
    history_ready INTEGER NOT NULL DEFAULT 0,
    card_message TEXT,
    card_text TEXT,
    PRIMARY KEY(bot,guild,channel),
    UNIQUE(bot,guild,instance,thread)
);

CREATE TABLE IF NOT EXISTS discord_categories (
    bot TEXT NOT NULL,
    guild TEXT NOT NULL,
    workspace TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('dispatching','accepted','outcome_unknown')),
    channel TEXT,
    PRIMARY KEY(bot,guild,workspace)
);

CREATE TABLE IF NOT EXISTS discord_deliveries (
    id TEXT PRIMARY KEY,
    bot TEXT NOT NULL,
    guild TEXT NOT NULL,
    channel TEXT NOT NULL,
    source_key TEXT NOT NULL,
    revision INTEGER NOT NULL,
    payload TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','dispatching','delivered','outcome_unknown','rejected')),
    not_before INTEGER NOT NULL DEFAULT 0,
    next_part INTEGER NOT NULL DEFAULT 0,
    rendered_parts TEXT,
    UNIQUE(bot,guild,channel,source_key)
);

CREATE TABLE IF NOT EXISTS discord_provisions (
    bot TEXT NOT NULL,
    guild TEXT NOT NULL,
    instance TEXT NOT NULL,
    thread TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('dispatching','accepted','outcome_unknown')),
    channel TEXT,
    PRIMARY KEY(bot,guild,instance,thread)
);

CREATE TABLE IF NOT EXISTS discord_ui_actions (
    nonce TEXT PRIMARY KEY,
    bot TEXT NOT NULL, guild TEXT NOT NULL, owner TEXT NOT NULL, channel TEXT NOT NULL,
    config_hash TEXT NOT NULL, action TEXT NOT NULL,
    expires INTEGER NOT NULL, consumed INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS discord_ui_actions_expiry ON discord_ui_actions(expires);

CREATE TABLE IF NOT EXISTS inbound_updates (
    bot_id INTEGER NOT NULL,
    polling_epoch INTEGER NOT NULL,
    update_id INTEGER NOT NULL,
    event_key TEXT NOT NULL,
    payload TEXT NOT NULL,
    received_at INTEGER NOT NULL,
    processed INTEGER NOT NULL DEFAULT 0,
    chat_id INTEGER NOT NULL DEFAULT 0,
    topic_id INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (bot_id, polling_epoch, update_id),
    UNIQUE (bot_id, event_key)
);

CREATE TABLE IF NOT EXISTS instances (
    id TEXT PRIMARY KEY,
    endpoint_hash TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS management_topics (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER,
    state TEXT NOT NULL CHECK(state IN ('dispatching','accepted','rejected','outcome_unknown')),
    CHECK(state!='accepted' OR topic_id>0),
    PRIMARY KEY(bot_id,chat_id)
);

CREATE TABLE IF NOT EXISTS operations (
    id TEXT PRIMARY KEY,
    source_key TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('received','prepared','dispatching','accepted','rejected','outcome_unknown')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
, payload TEXT, receipt TEXT);



CREATE TABLE IF NOT EXISTS polling_cursors (
    bot_id INTEGER PRIMARY KEY,
    polling_epoch INTEGER NOT NULL,
    next_offset INTEGER,
    generation_lower_bound INTEGER NOT NULL
, last_poll_at INTEGER NOT NULL DEFAULT 0);

CREATE TABLE IF NOT EXISTS topic_requests (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('dispatching','accepted','rejected','outcome_unknown')),
    topic_id INTEGER,
    PRIMARY KEY(bot_id, chat_id, instance_id, thread_id)
);

CREATE INDEX IF NOT EXISTS deliveries_pending ON deliveries(bot_id, state, not_before);

CREATE TRIGGER IF NOT EXISTS discord_binding_identity_immutable
BEFORE UPDATE OF instance,thread,workspace,cwd,endpoint,owner ON discord_bindings
BEGIN SELECT RAISE(ABORT,'Discord container identity is immutable'); END;

CREATE INDEX IF NOT EXISTS inbox_peers ON inbound_updates(bot_id,processed,chat_id,topic_id,received_at);

CREATE INDEX IF NOT EXISTS inbox_pending ON inbound_updates(bot_id, processed, received_at);

CREATE UNIQUE INDEX IF NOT EXISTS management_topic_address ON management_topics(bot_id,chat_id,topic_id);

CREATE TRIGGER IF NOT EXISTS management_topic_cannot_bind
BEFORE INSERT ON bindings
WHEN EXISTS(SELECT 1 FROM management_topics m WHERE m.bot_id=NEW.bot_id AND m.chat_id=NEW.chat_id AND m.topic_id=NEW.topic_id AND m.state='accepted')
BEGIN SELECT RAISE(ABORT,'management topic cannot bind an agent session'); END;

CREATE TRIGGER IF NOT EXISTS management_topic_cannot_use_session_container
BEFORE UPDATE OF topic_id,state ON management_topics
WHEN NEW.state='accepted' AND EXISTS(SELECT 1 FROM containers c WHERE c.bot_id=NEW.bot_id AND c.chat_id=NEW.chat_id AND c.topic_id=NEW.topic_id)
BEGIN SELECT RAISE(ABORT,'management topic cannot reuse a session container'); END;

CREATE UNIQUE INDEX IF NOT EXISTS one_active_binding_per_session
ON bindings(bot_id,chat_id,instance_id,thread_id) WHERE active=1;

CREATE UNIQUE INDEX IF NOT EXISTS catalog_endpoint_identity
ON catalog_providers(host_id, json_extract(payload, '$.endpoint.binary'), json_extract(payload, '$.endpoint.socket'));

CREATE UNIQUE INDEX IF NOT EXISTS catalog_workspace_identity
ON catalog_workspaces(provider_id, json_extract(payload, '$.cwd'));

CREATE UNIQUE INDEX IF NOT EXISTS catalog_workspace_short_name
ON catalog_workspaces(json_extract(payload, '$.short_name'));


CREATE TABLE IF NOT EXISTS navigation (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    id TEXT NOT NULL UNIQUE,
    payload TEXT NOT NULL,
    after_message_id INTEGER NOT NULL,
    message_id INTEGER,
    published_at INTEGER,
    consumed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(bot_id,chat_id,topic_id)
);

CREATE TABLE IF NOT EXISTS preferences (
    bot_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    locale TEXT NOT NULL CHECK (locale IN ('zh-CN','en')),
    PRIMARY KEY (bot_id,user_id)
);

CREATE TABLE IF NOT EXISTS topic_removals (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    request_key TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('dispatching','accepted','rejected','outcome_unknown')),
    PRIMARY KEY (bot_id,chat_id,topic_id)
);

CREATE TABLE IF NOT EXISTS slack_bindings (
    id TEXT PRIMARY KEY, app TEXT NOT NULL, team TEXT NOT NULL, bot TEXT NOT NULL,
    channel TEXT NOT NULL, root TEXT NOT NULL, owner TEXT NOT NULL,
    workspace TEXT NOT NULL, instance TEXT NOT NULL, thread TEXT NOT NULL,
    cwd TEXT NOT NULL, endpoint TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1,
    revision INTEGER NOT NULL DEFAULT 1, observed TEXT NOT NULL DEFAULT '{}',
    card TEXT NOT NULL DEFAULT '', history_ready INTEGER NOT NULL DEFAULT 0,
    UNIQUE(app,team,bot,channel,root), UNIQUE(app,team,bot,channel,instance,thread)
);
CREATE TRIGGER IF NOT EXISTS slack_binding_identity_immutable
BEFORE UPDATE OF app,team,bot,channel,root,owner,workspace,instance,thread,cwd,endpoint ON slack_bindings
BEGIN SELECT RAISE(ABORT,'Slack container identity is immutable'); END;
CREATE TABLE IF NOT EXISTS slack_provisions (
    app TEXT NOT NULL, team TEXT NOT NULL, bot TEXT NOT NULL, channel TEXT NOT NULL,
    instance TEXT NOT NULL, thread TEXT NOT NULL, state TEXT NOT NULL,
    PRIMARY KEY(app,team,bot,channel,instance,thread)
);
CREATE TABLE IF NOT EXISTS slack_inbox (
    id TEXT PRIMARY KEY, namespace TEXT NOT NULL, source TEXT NOT NULL,
    request TEXT NOT NULL, config_hash TEXT NOT NULL, state TEXT NOT NULL,
    created INTEGER NOT NULL DEFAULT (unixepoch()), UNIQUE(namespace,source)
);
CREATE TABLE IF NOT EXISTS slack_actions (
    nonce TEXT PRIMARY KEY, namespace TEXT NOT NULL, channel TEXT NOT NULL,
    owner TEXT NOT NULL, root TEXT, binding TEXT, revision INTEGER,
    action TEXT NOT NULL, config_hash TEXT NOT NULL, expires INTEGER NOT NULL,
    consumed INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS slack_outbox (
    id TEXT PRIMARY KEY, namespace TEXT NOT NULL, channel TEXT NOT NULL,
    root TEXT, binding TEXT, revision INTEGER,
    source TEXT NOT NULL, content TEXT NOT NULL, config_hash TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending', not_before INTEGER NOT NULL DEFAULT 0,
    next_part INTEGER NOT NULL DEFAULT 0, rendered_parts TEXT, UNIQUE(namespace,source)
);
CREATE INDEX IF NOT EXISTS slack_outbox_pending ON slack_outbox(namespace,state,not_before);
CREATE TABLE IF NOT EXISTS slack_channel_budget (
    namespace TEXT NOT NULL, channel TEXT NOT NULL, next_send INTEGER NOT NULL,
    PRIMARY KEY(namespace,channel)
);

-- PocketPilot-owned runtime server state. Its database is separate from the chat bridge ledger.
CREATE TABLE IF NOT EXISTS runtime_sessions (
    id TEXT PRIMARY KEY, payload TEXT NOT NULL, native_started INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS runtime_items (
    session TEXT NOT NULL REFERENCES runtime_sessions(id), key TEXT NOT NULL,
    payload TEXT NOT NULL, ordinal INTEGER NOT NULL,
    PRIMARY KEY(session,key)
);
CREATE TABLE IF NOT EXISTS runtime_operations (
    id TEXT PRIMARY KEY, digest TEXT NOT NULL, state TEXT NOT NULL,
    receipt TEXT
);

-- Runtime uploads are scoped to the managed session, not merely a guessed pathname.
CREATE TABLE IF NOT EXISTS runtime_uploads (
    path TEXT PRIMARY KEY,
    session TEXT NOT NULL REFERENCES runtime_sessions(id)
);
