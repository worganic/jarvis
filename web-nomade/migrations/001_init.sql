-- Jarvis Nomade — schéma initial. Les dates sont en millisecondes (comme dans Jarvis).

CREATE TABLE IF NOT EXISTS users (
  id INT AUTO_INCREMENT PRIMARY KEY,
  login VARCHAR(80) NOT NULL UNIQUE,
  password_hash VARCHAR(255) NOT NULL,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Connexions mémorisées du site : jeton « sélecteur:validateur », seul le validateur haché est gardé.
CREATE TABLE IF NOT EXISTS sessions (
  id INT AUTO_INCREMENT PRIMARY KEY,
  user_id INT NOT NULL,
  selector CHAR(24) NOT NULL UNIQUE,
  validator_hash CHAR(64) NOT NULL,
  label VARCHAR(160) NOT NULL DEFAULT '',
  ip VARCHAR(64) NOT NULL DEFAULT '',
  created_at BIGINT NOT NULL,
  last_seen BIGINT NOT NULL,
  expires_at BIGINT NOT NULL,
  KEY idx_sessions_user (user_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Appareils Jarvis (PC) : clé d'API hachée.
CREATE TABLE IF NOT EXISTS devices (
  id INT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(80) NOT NULL DEFAULT 'Jarvis',
  key_hash CHAR(64) NOT NULL UNIQUE,
  version VARCHAR(20) NOT NULL DEFAULT '',
  created_at BIGINT NOT NULL,
  last_seen BIGINT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Sources partagées par Jarvis (une liste à fiches, les notes, les rappels…).
CREATE TABLE IF NOT EXISTS sources (
  id VARCHAR(80) PRIMARY KEY,
  label VARCHAR(160) NOT NULL,
  grp VARCHAR(60) NOT NULL DEFAULT '',
  kind VARCHAR(30) NOT NULL DEFAULT '',
  ops VARCHAR(120) NOT NULL DEFAULT '',
  meta LONGTEXT NULL,
  position INT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Enregistrements, format générique : une source de plus = aucune migration.
CREATE TABLE IF NOT EXISTS records (
  source VARCHAR(80) NOT NULL,
  rid VARCHAR(120) NOT NULL,
  data LONGTEXT NOT NULL,
  hash CHAR(64) NOT NULL DEFAULT '',
  rev INT NOT NULL DEFAULT 1,
  origin VARCHAR(8) NOT NULL DEFAULT 'pc',
  updated_at BIGINT NOT NULL,
  deleted_at BIGINT NULL,
  PRIMARY KEY (source, rid),
  KEY idx_records_source (source, deleted_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Journal de synchronisation : Jarvis lit les changements « web » après son dernier numéro.
CREATE TABLE IF NOT EXISTS changes (
  seq BIGINT AUTO_INCREMENT PRIMARY KEY,
  source VARCHAR(80) NOT NULL,
  rid VARCHAR(120) NOT NULL,
  op VARCHAR(12) NOT NULL,
  origin VARCHAR(8) NOT NULL,
  patch LONGTEXT NULL,
  at BIGINT NOT NULL,
  KEY idx_changes_origin (origin, seq)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Boîte de dépôt : ce qui est écrit dehors pour Jarvis.
CREATE TABLE IF NOT EXISTS inbox (
  id INT AUTO_INCREMENT PRIMARY KEY,
  kind VARCHAR(20) NOT NULL DEFAULT 'note',
  text TEXT NOT NULL,
  state VARCHAR(10) NOT NULL DEFAULT 'nouveau',
  label VARCHAR(160) NOT NULL DEFAULT '',
  created_at BIGINT NOT NULL,
  taken_at BIGINT NULL,
  KEY idx_inbox_state (state)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Tentatives de connexion refusées (limite par adresse IP).
CREATE TABLE IF NOT EXISTS attempts (
  id INT AUTO_INCREMENT PRIMARY KEY,
  ip VARCHAR(64) NOT NULL,
  at BIGINT NOT NULL,
  KEY idx_attempts_ip (ip, at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
