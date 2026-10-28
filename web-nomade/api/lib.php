<?php
// Jarvis Nomade — outils communs de l'API : base, réponses JSON, authentification (site et Jarvis).

declare(strict_types=1);

const SESSION_COOKIE = 'nomade_session';
const SESSION_DAYS = 90;
const MAX_ATTEMPTS = 8;
const ATTEMPT_WINDOW_MS = 15 * 60 * 1000;
const MAX_BODY = 4 * 1024 * 1024;

function cfg(): array {
    static $c = null;
    if ($c === null) {
        $file = __DIR__ . '/../private/config.php';
        if (!is_file($file)) {
            fail(503, 'Site non configuré (private/config.php absent).');
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
        fail(413, 'Requête trop volumineuse.');
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

function client_ip(): string {
    return substr((string)($_SERVER['REMOTE_ADDR'] ?? ''), 0, 64);
}

function sha(string $s): string {
    return hash('sha256', $s);
}

function token(int $bytes): string {
    return rtrim(strtr(base64_encode(random_bytes($bytes)), '+/', '-_'), '=');
}

function hash_password(string $p): string {
    return password_hash($p, defined('PASSWORD_ARGON2ID') ? PASSWORD_ARGON2ID : PASSWORD_BCRYPT);
}

// --- Limite des tentatives ------------------------------------------------------------------------

function check_attempts(): void {
    $st = db()->prepare('SELECT COUNT(*) FROM attempts WHERE ip = ? AND at > ?');
    $st->execute([client_ip(), now_ms() - ATTEMPT_WINDOW_MS]);
    if ((int)$st->fetchColumn() >= MAX_ATTEMPTS) {
        fail(429, 'Trop de tentatives : réessaie dans 15 minutes.');
    }
}

function note_attempt(): void {
    db()->prepare('INSERT INTO attempts (ip, at) VALUES (?, ?)')->execute([client_ip(), now_ms()]);
    db()->prepare('DELETE FROM attempts WHERE at < ?')->execute([now_ms() - 24 * 3600 * 1000]);
}

function check_credentials(string $login, string $password): ?array {
    $st = db()->prepare('SELECT * FROM users WHERE login = ?');
    $st->execute([$login]);
    $u = $st->fetch();
    if (!$u || $u['password_hash'] === null || !password_verify($password, $u['password_hash'])) {
        return null;
    }
    if (password_needs_rehash($u['password_hash'], defined('PASSWORD_ARGON2ID') ? PASSWORD_ARGON2ID : PASSWORD_BCRYPT)) {
        db()->prepare('UPDATE users SET password_hash = ? WHERE id = ?')->execute([hash_password($password), $u['id']]);
    }
    return $u;
}

// --- Session du site (cookie mémorisé) ------------------------------------------------------------

function set_session_cookie(string $value, int $expires_ms): void {
    setcookie(SESSION_COOKIE, $value, [
        'expires' => intdiv($expires_ms, 1000),
        'path' => '/',
        'secure' => true,
        'httponly' => true,
        'samesite' => 'Strict',
    ]);
}

function open_session(int $user_id, string $label): void {
    $selector = token(18);
    $validator = token(32);
    $now = now_ms();
    $expires = $now + SESSION_DAYS * 86400000;
    db()->prepare('INSERT INTO sessions (user_id, selector, validator_hash, label, ip, created_at, last_seen, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
        ->execute([$user_id, $selector, sha($validator), mb_substr($label, 0, 160), client_ip(), $now, $now, $expires]);
    set_session_cookie("$selector:$validator", $expires);
}

/** Session du site, prolongée à chaque visite. Sans session valide : null. */
function web_session(): ?array {
    $raw = (string)($_COOKIE[SESSION_COOKIE] ?? '');
    if (!preg_match('/^([\w-]{24}):([\w-]{20,})$/', $raw, $m)) {
        return null;
    }
    $st = db()->prepare('SELECT s.*, u.login, u.role, u.mode, u.label AS user_label FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.selector = ?');
    $st->execute([$m[1]]);
    $s = $st->fetch();
    if (!$s || !hash_equals($s['validator_hash'], sha($m[2])) || (int)$s['expires_at'] < now_ms()) {
        return null;
    }
    $now = now_ms();
    if ($now - (int)$s['last_seen'] > 3600000) {
        $expires = $now + SESSION_DAYS * 86400000;
        db()->prepare('UPDATE sessions SET last_seen = ?, expires_at = ?, ip = ? WHERE id = ?')->execute([$now, $expires, client_ip(), $s['id']]);
        set_session_cookie($raw, $expires);
    }
    return $s;
}

function require_web(): array {
    $s = web_session();
    if (!$s) {
        fail(401, 'Connexion requise.');
    }
    // Requêtes d'écriture : en-tête propre au site (une autre origine ne peut pas l'ajouter sans CORS).
    if ($_SERVER['REQUEST_METHOD'] !== 'GET' && ($_SERVER['HTTP_X_NOMADE'] ?? '') !== '1') {
        fail(403, 'Requête refusée.');
    }
    return $s;
}

// --- Appareil Jarvis (clé d'API) ------------------------------------------------------------------

function require_device(): array {
    $h = (string)($_SERVER['HTTP_AUTHORIZATION'] ?? $_SERVER['REDIRECT_HTTP_AUTHORIZATION'] ?? '');
    if (!preg_match('/^Bearer\s+([\w-]{30,})$/', $h, $m)) {
        fail(401, 'Clé Jarvis absente.');
    }
    $st = db()->prepare('SELECT * FROM devices WHERE key_hash = ?');
    $st->execute([sha($m[1])]);
    $d = $st->fetch();
    if (!$d) {
        fail(401, 'Clé Jarvis inconnue.');
    }
    $version = substr((string)($_SERVER['HTTP_X_JARVIS_VERSION'] ?? ''), 0, 20);
    db()->prepare('UPDATE devices SET last_seen = ?, version = ? WHERE id = ?')->execute([now_ms(), $version, $d['id']]);
    return $d;
}

// --- Données --------------------------------------------------------------------------------------

function source_row(string $id): ?array {
    $st = db()->prepare('SELECT * FROM sources WHERE id = ?');
    $st->execute([$id]);
    $r = $st->fetch();
    return $r ?: null;
}

function public_source(array $s): array {
    return [
        'id' => $s['id'], 'label' => $s['label'], 'group' => $s['grp'], 'kind' => $s['kind'],
        'ops' => array_values(array_filter(explode(',', $s['ops']))),
        'meta' => $s['meta'] ? json_decode($s['meta'], true) : null, 'updatedAt' => (int)$s['updated_at'],
        'ownerUser' => isset($s['owner_user']) ? (int)$s['owner_user'] : null,
        'creatorUser' => creator_of($s),
    ];
}

// --- Listes des invités ---------------------------------------------------------------------------

const GUEST_LIST_PREFIX = 'ami:';
const GUEST_LIST_OPS = 'add,update,check,delete';
const MAX_GUEST_LISTS = 30;
const RELATIONS = ['famille', 'ami', 'collegue', 'autre'];

/** Liste d'invité que la session peut gérer (renommer, supprimer) : la sienne, ou toutes pour le propriétaire. */
function guest_list_for(array $s, string $id): array {
    $src = source_row($id);
    if (!$src || $src['owner_user'] === null || (!is_owner($s) && (int)$src['owner_user'] !== (int)$s['user_id'])) {
        fail(404, 'Liste introuvable.');
    }
    return $src;
}

/** Suppression d'une liste d'invité et de son contenu ; Jarvis l'apprend par le journal (op « list »). */
function drop_guest_list(string $id): void {
    foreach (['DELETE FROM records WHERE source = ?', 'DELETE FROM changes WHERE source = ?', 'DELETE FROM sources WHERE id = ?'] as $sql) {
        db()->prepare($sql)->execute([$id]);
    }
    log_change($id, '', 'list', 'web');
}

function log_change(string $source, string $rid, string $op, string $origin, ?array $patch = null, string $author = ''): int {
    db()->prepare('INSERT INTO changes (source, rid, op, origin, patch, author, at) VALUES (?, ?, ?, ?, ?, ?, ?)')
        ->execute([$source, $rid, $op, $origin, $patch === null ? null : json_encode($patch, JSON_UNESCAPED_UNICODE), $author, now_ms()]);
    return (int)db()->lastInsertId();
}

function last_seq(): int {
    return (int)db()->query('SELECT COALESCE(MAX(seq), 0) FROM changes')->fetchColumn();
}

function jarvis_seen(): array {
    $r = db()->query('SELECT name, version, last_seen FROM devices ORDER BY last_seen DESC LIMIT 1')->fetch();
    return $r ? ['name' => $r['name'], 'version' => $r['version'], 'lastSeen' => (int)$r['last_seen']] : ['lastSeen' => 0];
}

// --- Images ---------------------------------------------------------------------------------------

const MAX_FILE = 2500000;

function files_dir(): string {
    return __DIR__ . '/../private/files';
}

/** « <sha256>.<ext> » : rien d'autre ne peut être lu ou écrit dans le dossier des images. */
function valid_file_name(string $n): bool {
    return (bool)preg_match('/^[a-f0-9]{64}\.(jpg|png|gif|webp)$/', $n);
}

// --- Partages -------------------------------------------------------------------------------------

const INVITE_DAYS = 7;

function is_owner(array $s): bool {
    return ($s['role'] ?? 'owner') === 'owner';
}

/** Sources visibles par la session : toutes pour le propriétaire, celles du partage pour un invité. */
function visible_source_ids(array $s): ?array {
    if (is_owner($s)) {
        return null; // toutes
    }
    // Sources partagées avec lui, et ses propres listes.
    $st = db()->prepare('SELECT source FROM user_sources WHERE user_id = ? UNION SELECT id FROM sources WHERE owner_user = ?');
    $st->execute([$s['user_id'], $s['user_id']]);
    return $st->fetchAll(PDO::FETCH_COLUMN);
}

function can_see(array $s, string $source): bool {
    $ids = visible_source_ids($s);
    return $ids === null || in_array($source, $ids, true);
}

/** Modifications permises à cette session sur une source (aucune pour un invité en lecture seule). */
function ops_for(array $s, array $src): array {
    $creator = creator_of($src);
    $mine = $creator !== null && $creator === (int)$s['user_id'];
    // Liste créée par un invité : son créateur décide, pour chaque personne, d'aucun accès, de la lecture ou de l'écriture.
    if (!is_owner($s) && !$mine && $creator !== null) {
        return write_granted($s, (string)$src['id']) ? array_values(array_filter(explode(',', $src['ops']))) : [];
    }
    if (!is_owner($s) && !$mine && ($s['mode'] ?? '') !== 'ecriture' && !write_granted($s, (string)$src['id'])) {
        return [];
    }
    return array_values(array_filter(explode(',', $src['ops'])));
}

/** Invité qui a créé la liste : propriétaire d'une liste simple (owner_user) ou créateur d'une liste à fiches demandée (creator_user). */
function creator_of(array $src): ?int {
    foreach (['creator_user', 'owner_user'] as $k) {
        if (isset($src[$k]) && $src[$k] !== null) {
            return (int)$src[$k];
        }
    }
    return null;
}

/** Écriture accordée à cet invité sur cette source seulement (liste créée à sa demande : user_sources.write_ok). */
function write_granted(array $s, string $source): bool {
    try {
        $st = db()->prepare('SELECT write_ok FROM user_sources WHERE user_id = ? AND source = ?');
        $st->execute([$s['user_id'], $source]);
        return (int)$st->fetchColumn() === 1;
    } catch (PDOException $e) {
        return false; // migration 004 pas encore appliquée
    }
}

/** Liste dont cette session est la créatrice (invité) : seule elle règle son partage. */
function shareable_list(array $s, string $id): array {
    $src = source_row($id);
    if (!$src || creator_of($src) === null || creator_of($src) !== (int)$s['user_id']) {
        fail(404, 'Liste introuvable ou partage réservé à son créateur.');
    }
    return $src;
}

function valid_login_name(string $login): bool {
    return (bool)preg_match('/^[\w.@-]{3,80}$/u', $login);
}

function require_owner_web(): array {
    $s = require_web();
    if (!is_owner($s)) {
        fail(403, 'Réservé au propriétaire du site.');
    }
    return $s;
}

/** Invitation : jeton remis une seule fois (dans le lien du mail), seul son haché est gardé. */
function new_invite(int $user_id): string {
    $t = token(24);
    db()->prepare('UPDATE users SET invite_hash = ?, invite_expires = ? WHERE id = ?')->execute([sha($t), now_ms() + INVITE_DAYS * 86400000, $user_id]);
    return $t;
}

function invited_user(string $login, string $tok): ?array {
    $st = db()->prepare("SELECT * FROM users WHERE login = ? AND role = 'guest' AND invite_hash IS NOT NULL");
    $st->execute([$login]);
    $u = $st->fetch();
    if (!$u || !hash_equals((string)$u['invite_hash'], sha($tok)) || (int)$u['invite_expires'] < now_ms()) {
        return null;
    }
    return $u;
}

function guest_row(array $u): array {
    $st = db()->prepare('SELECT source FROM user_sources WHERE user_id = ?');
    $st->execute([$u['id']]);
    $seen = db()->prepare('SELECT MAX(last_seen) FROM sessions WHERE user_id = ?');
    $seen->execute([$u['id']]);
    return [
        'id' => (int)$u['id'], 'login' => $u['login'], 'email' => $u['email'], 'label' => $u['label'], 'mode' => $u['mode'], 'relation' => $u['relation'] ?? 'ami',
        'lists' => (int)db()->query('SELECT COUNT(*) FROM sources WHERE owner_user = ' . (int)$u['id'])->fetchColumn(),
        'active' => $u['password_hash'] !== null, 'invitePending' => $u['invite_hash'] !== null,
        'inviteExpires' => $u['invite_expires'] === null ? null : (int)$u['invite_expires'],
        'sources' => $st->fetchAll(PDO::FETCH_COLUMN), 'lastSeen' => (int)$seen->fetchColumn(), 'createdAt' => (int)$u['created_at'],
    ];
}
