<?php
// Jarvis Réseau — outils communs de l'API : base, réponses JSON, appareil authentifié, limites de débit, mails.
// Repris du site Nomade (web-nomade/api/lib.php), en multi-comptes : chaque Jarvis a son compte (mail vérifié).

declare(strict_types=1);

const MAX_BODY = 3 * 1024 * 1024;       // un envoi : 2 Mo de contenu chiffré au plus (base64 compris)
const MAX_ITEM = 2 * 1024 * 1024;
const MAX_PENDING_BYTES = 40 * 1024 * 1024; // en attente de relève, par destinataire
const ITEM_DAYS = 30;                    // un envoi jamais relevé est effacé au bout de 30 jours
const CODE_MINUTES = 15;

function cfg(): array {
    static $c = null;
    if ($c === null) {
        $file = __DIR__ . '/../private/config.php';
        if (!is_file($file)) {
            fail(503, 'Service non configuré (private/config.php absent).');
        }
        $c = require $file;
    }
    return $c;
}

function db(): PDO {
    static $pdo = null;
    if ($pdo === null) {
        $c = cfg();
        $dsn = sprintf('mysql:host=%s;port=%d;dbname=%s;charset=utf8mb4', $c['db_host'], (int)($c['db_port'] ?? 3306), $c['db_name']);
        try {
            $pdo = new PDO($dsn, $c['db_user'], $c['db_password'], [
                PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
                PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC,
                PDO::ATTR_EMULATE_PREPARES => false,
            ]);
        } catch (PDOException $e) {
            fail(503, 'Base de données injoignable.');
        }
    }
    return $pdo;
}

function now_ms(): int {
    return (int)floor(microtime(true) * 1000);
}

function send(array $data, int $status = 200): void {
    http_response_code($status);
    header('Content-Type: application/json; charset=utf-8');
    header('Cache-Control: no-store');
    echo json_encode($data, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
    exit;
}

function fail(int $status, string $error): void {
    send(['ok' => false, 'error' => $error], $status);
}

function body(): array {
    $raw = file_get_contents('php://input', false, null, 0, MAX_BODY + 1);
    if ($raw === false || $raw === '') {
        return [];
    }
    if (strlen($raw) > MAX_BODY) {
        fail(413, 'Envoi trop volumineux.');
    }
    $j = json_decode($raw, true);
    if (!is_array($j)) {
        fail(400, 'JSON invalide.');
    }
    return $j;
}

function str_arg(array $a, string $k, int $max = 2000): string {
    $v = $a[$k] ?? '';
    if (!is_string($v) && !is_numeric($v)) {
        return '';
    }
    return mb_substr(trim((string)$v), 0, $max);
}

function int_arg(array $a, string $k): int {
    return (int)($a[$k] ?? 0);
}

function client_ip(): string {
    return substr((string)($_SERVER['REMOTE_ADDR'] ?? ''), 0, 64);
}

function sha(string $s): string {
    return hash('sha256', $s);
}

function token(int $bytes): string {
    return rtrim(strtr(base64_encode(random_bytes($bytes)), '+/', '-_'), '=');
}

function valid_email(string $e): bool {
    return strlen($e) <= 190 && (bool)filter_var($e, FILTER_VALIDATE_EMAIL);
}

/// Clé publique X25519 en base64 (32 octets).
function valid_key(string $k): bool {
    $raw = base64_decode($k, true);
    return $raw !== false && strlen($raw) === 32;
}

// --- Limites de débit ---------------------------------------------------------------------------------------------------

/// Refuse au-delà de `max` actions de `bucket` sur `window_ms`, sinon compte celle-ci.
function rate(string $bucket, int $max, int $window_ms, string $message): void {
    $st = db()->prepare('SELECT COUNT(*) FROM res_rate WHERE bucket = ? AND at > ?');
    $st->execute([$bucket, now_ms() - $window_ms]);
    if ((int)$st->fetchColumn() >= $max) {
        fail(429, $message);
    }
    db()->prepare('INSERT INTO res_rate (bucket, at) VALUES (?, ?)')->execute([$bucket, now_ms()]);
    if (random_int(1, 50) === 1) {
        db()->prepare('DELETE FROM res_rate WHERE at < ?')->execute([now_ms() - 7 * 86400 * 1000]);
        db()->prepare('DELETE FROM res_items WHERE created_at < ?')->execute([now_ms() - ITEM_DAYS * 86400 * 1000]);
        db()->prepare('DELETE FROM res_email_codes WHERE expires_at < ?')->execute([now_ms()]);
    }
}

// --- Appareil (Jarvis) ----------------------------------------------------------------------------------------------------

/// Compte de l'appareil qui appelle (en-tête Authorization: Bearer <clé>).
function require_account(): array {
    $h = (string)($_SERVER['HTTP_AUTHORIZATION'] ?? $_SERVER['REDIRECT_HTTP_AUTHORIZATION'] ?? '');
    if (!preg_match('/^Bearer\s+([\w-]{30,})$/', $h, $m)) {
        fail(401, 'Clé Jarvis absente.');
    }
    $st = db()->prepare('SELECT a.*, d.id AS device_id FROM res_devices d JOIN res_accounts a ON a.id = d.account_id WHERE d.key_hash = ?');
    $st->execute([sha($m[1])]);
    $a = $st->fetch();
    if (!$a) {
        fail(401, 'Clé Jarvis inconnue (compte supprimé ou appareil retiré).');
    }
    $version = substr((string)($_SERVER['HTTP_X_JARVIS_VERSION'] ?? ''), 0, 20);
    db()->prepare('UPDATE res_devices SET last_seen = ?, version = ? WHERE id = ?')->execute([now_ms(), $version, $a['device_id']]);
    db()->prepare('UPDATE res_accounts SET last_seen = ? WHERE id = ?')->execute([now_ms(), $a['id']]);
    return $a;
}

function account_by_email(string $email): ?array {
    $st = db()->prepare('SELECT * FROM res_accounts WHERE email = ?');
    $st->execute([$email]);
    return $st->fetch() ?: null;
}

// --- Amitiés et partages ------------------------------------------------------------------------------------------------

/// Relation entre deux comptes (dans un sens ou dans l'autre), ou null.
function friendship(int $a, int $b): ?array {
    $st = db()->prepare('SELECT * FROM res_friendships WHERE (requester_id = ? AND target_id = ?) OR (requester_id = ? AND target_id = ?)');
    $st->execute([$a, $b, $b, $a]);
    return $st->fetch() ?: null;
}

function are_friends(int $a, int $b): bool {
    $f = friendship($a, $b);
    return $f !== null && $f['status'] === 'accepted';
}

function shared(int $owner, int $friend, string $type, string $ref = ''): ?array {
    $st = db()->prepare('SELECT * FROM res_shares WHERE owner_id = ? AND friend_id = ? AND type = ? AND ref = ?');
    $st->execute([$owner, $friend, $type, $ref]);
    return $st->fetch() ?: null;
}

// --- Mails --------------------------------------------------------------------------------------------------------------

function mail_to(string $to, string $subject, string $text): bool {
    $from = (string)(cfg()['mail_from'] ?? '');
    if ($from === '') {
        return false;
    }
    $headers = implode("\r\n", [
        'From: Jarvis <' . $from . '>',
        'Content-Type: text/plain; charset=utf-8',
        'Content-Transfer-Encoding: 8bit',
        'X-Mailer: Jarvis Reseau',
    ]);
    return @mail($to, '=?UTF-8?B?' . base64_encode($subject) . '?=', $text, $headers);
}
