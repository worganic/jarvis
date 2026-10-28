-- Liste créée à la demande d'un invité : il peut la modifier même si son accès est en lecture seule ailleurs.
-- 1 = écriture accordée sur cette source (route guest-grant, posée par Jarvis).

ALTER TABLE user_sources ADD COLUMN write_ok TINYINT NOT NULL DEFAULT 0;
