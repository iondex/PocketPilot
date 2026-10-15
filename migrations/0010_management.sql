ALTER TABLE bindings ADD COLUMN topic_title TEXT;
CREATE TABLE management_topics (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER,
    state TEXT NOT NULL CHECK(state IN ('dispatching','accepted','rejected','outcome_unknown')),
    CHECK(state!='accepted' OR topic_id>0),
    PRIMARY KEY(bot_id,chat_id)
);
CREATE UNIQUE INDEX management_topic_address ON management_topics(bot_id,chat_id,topic_id);
CREATE TRIGGER management_topic_cannot_bind
BEFORE INSERT ON bindings
WHEN EXISTS(SELECT 1 FROM management_topics m WHERE m.bot_id=NEW.bot_id AND m.chat_id=NEW.chat_id AND m.topic_id=NEW.topic_id AND m.state='accepted')
BEGIN SELECT RAISE(ABORT,'management topic cannot bind an agent session'); END;
CREATE TRIGGER management_topic_cannot_use_session_container
BEFORE UPDATE OF topic_id,state ON management_topics
WHEN NEW.state='accepted' AND EXISTS(SELECT 1 FROM containers c WHERE c.bot_id=NEW.bot_id AND c.chat_id=NEW.chat_id AND c.topic_id=NEW.topic_id)
BEGIN SELECT RAISE(ABORT,'management topic cannot reuse a session container'); END;
