-- Partages : comptes invités (lecture seule ou lecture/écriture) limités à certaines sources.
-- Un invité choisit son mot de passe à sa première visite (lien d'invitation, jeton haché, durée limitée).

ALTER TABLE users ADD COLUMN role VARCHAR(10) NOT NULL DEFAULT 'owner';
ALTER TABLE users ADD COLUMN email VARCHAR(190) NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN label VARCHAR(120) NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN mode VARCHAR(10) NOT NULL DEFAULT 'ecriture';
ALTER TABLE users ADD COLUMN invite_hash CHAR(64) NULL;
ALTER TABLE users ADD COLUMN invite_expires BIGINT NULL;
ALTER TABLE users MODIFY password_hash VARCHAR(255) NULL;

CREATE TABLE IF NOT EXISTS user_sources (
  user_id INT NOT NULL,
  source VARCHAR(80) NOT NULL,
  PRIMARY KEY (user_id, source),
  KEY idx_user_sources_source (source)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Qui a fait la modification ou le dépôt (identifiant du site).
ALTER TABLE changes ADD COLUMN author VARCHAR(80) NOT NULL DEFAULT '';
ALTER TABLE inbox ADD COLUMN author VARCHAR(80) NOT NULL DEFAULT '';
