-- Jarvis Réseau : comptes (un mail vérifié), appareils, amitiés, partages, boîte aux lettres chiffrée, présence.
-- Préfixe res_ : mêmes base et hébergement que le site Nomade, dont aucune table n'est touchée.
-- Le contenu échangé (messages, listes, fichiers) est chiffré de bout en bout par les Jarvis : le serveur ne le lit pas.

CREATE TABLE IF NOT EXISTS res_accounts (
  id INT AUTO_INCREMENT PRIMARY KEY,
  email VARCHAR(190) NOT NULL UNIQUE,
  name VARCHAR(80) NOT NULL,
  public_key VARCHAR(64) NOT NULL,
  created_at BIGINT NOT NULL,
  last_seen BIGINT NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS res_devices (
  id INT AUTO_INCREMENT PRIMARY KEY,
  account_id INT NOT NULL,
  key_hash CHAR(64) NOT NULL UNIQUE,
  label VARCHAR(120) NOT NULL DEFAULT '',
  version VARCHAR(20) NOT NULL DEFAULT '',
  created_at BIGINT NOT NULL,
  last_seen BIGINT NOT NULL DEFAULT 0,
  INDEX (account_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS res_email_codes (
  email VARCHAR(190) NOT NULL PRIMARY KEY,
  code_hash CHAR(64) NOT NULL,
  name VARCHAR(80) NOT NULL,
  public_key VARCHAR(64) NOT NULL,
  expires_at BIGINT NOT NULL,
  tries INT NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Une ligne par paire (requester → target) ; status : pending, accepted, refused, blocked (blocked_by : qui a bloqué).
CREATE TABLE IF NOT EXISTS res_friendships (
  id INT AUTO_INCREMENT PRIMARY KEY,
  requester_id INT NOT NULL,
  target_id INT NOT NULL,
  status VARCHAR(10) NOT NULL,
  blocked_by INT NULL,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL,
  UNIQUE KEY pair (requester_id, target_id),
  INDEX (target_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Invitations envoyées à un mail qui n'a pas encore de compte : la demande d'ami naît à son inscription.
CREATE TABLE IF NOT EXISTS res_invites (
  id INT AUTO_INCREMENT PRIMARY KEY,
  requester_id INT NOT NULL,
  email VARCHAR(190) NOT NULL,
  created_at BIGINT NOT NULL,
  UNIQUE KEY pair (requester_id, email)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Boîte aux lettres : kind (message, lu, liste, liste-op, fichier, presence-ping…), payload chiffré pour le destinataire.
CREATE TABLE IF NOT EXISTS res_items (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  sender_id INT NOT NULL,
  recipient_id INT NOT NULL,
  kind VARCHAR(20) NOT NULL,
  ref VARCHAR(120) NOT NULL DEFAULT '',
  nonce VARCHAR(40) NOT NULL,
  payload MEDIUMTEXT NOT NULL,
  bytes INT NOT NULL,
  created_at BIGINT NOT NULL,
  INDEX inbox (recipient_id, id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Présence visible des seuls amis à qui elle est partagée : état (en_ligne, absent, ne_pas_deranger) et stade du compagnon.
CREATE TABLE IF NOT EXISTS res_presence (
  account_id INT PRIMARY KEY,
  state VARCHAR(20) NOT NULL,
  avatar VARCHAR(200) NOT NULL DEFAULT '',
  updated_at BIGINT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Ce que chaque compte partage avec chaque ami (type : presence, liste, dossier ; ref : identifiant local ; rights : lecture, ecriture).
CREATE TABLE IF NOT EXISTS res_shares (
  id INT AUTO_INCREMENT PRIMARY KEY,
  owner_id INT NOT NULL,
  friend_id INT NOT NULL,
  type VARCHAR(12) NOT NULL,
  ref VARCHAR(120) NOT NULL DEFAULT '',
  label VARCHAR(160) NOT NULL DEFAULT '',
  rights VARCHAR(10) NOT NULL DEFAULT 'lecture',
  created_at BIGINT NOT NULL,
  UNIQUE KEY share (owner_id, friend_id, type, ref),
  INDEX (friend_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Limites de débit (inscription, demandes d'ami, envois) par compte ou par adresse IP.
CREATE TABLE IF NOT EXISTS res_rate (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  bucket VARCHAR(120) NOT NULL,
  at BIGINT NOT NULL,
  INDEX (bucket, at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
