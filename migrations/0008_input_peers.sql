ALTER TABLE inbound_updates ADD COLUMN chat_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE inbound_updates ADD COLUMN topic_id INTEGER NOT NULL DEFAULT 0;
UPDATE inbound_updates SET chat_id=json_extract(payload,'$.chat'),topic_id=json_extract(payload,'$.topic');
CREATE INDEX inbox_peers ON inbound_updates(bot_id,processed,chat_id,topic_id,received_at);
