<?php
// Jarvis Nomade — API. Route : api/?r=<nom>. Le site utilise le cookie de session, Jarvis sa clé d'appareil.

declare(strict_types=1);
require __DIR__ . '/lib.php';

header('X-Content-Type-Options: nosniff');
header('Referrer-Policy: no-referrer');

$route = preg_replace('/[^a-z-]/', '', (string)($_GET['r'] ?? ''));
$method = $_SERVER['REQUEST_METHOD'];
$WEB_OPS = ['add', 'update', 'status', 'check', 'delete'];

switch ("$method $route") {

    // --- Site ---------------------------------------------------------------------------------

    case 'POST login': {
        $a = body();
        check_attempts();
        $u = check_credentials(str_arg($a, 'login', 80), (string)($a['password'] ?? ''));
        if (!$u) {
            note_attempt();
            fail(401, 'Identifiant ou mot de passe incorrect.');
        }
        open_session((int)$u['id'], str_arg($a, 'label', 160) ?: substr((string)($_SERVER['HTTP_USER_AGENT'] ?? ''), 0, 160));
        send(['ok' => true]);
    }

    case 'POST logout': {
        $s = web_session();
        if ($s) {
            db()->prepare('DELETE FROM sessions WHERE id = ?')->execute([$s['id']]);
        }
        set_session_cookie('', 1000);
        send(['ok' => true]);
    }

    case 'GET me': {
        $s = require_web();
        $visible = visible_source_ids($s);
        $rows = array_filter(db()->query('SELECT * FROM sources ORDER BY position, label')->fetchAll(), fn($r) => $visible === null || in_array($r['id'], $visible, true));
        $names = [];
        foreach (db()->query("SELECT id, label, login FROM users WHERE role = 'guest'")->fetchAll() as $u) {
            $names[(int)$u['id']] = $u['label'] ?: $u['login'];
        }
        $sources = array_map(fn($r) => array_merge(public_source($r), [
            'ops' => ops_for($s, $r),
            'mine' => $r['owner_user'] !== null && (int)$r['owner_user'] === (int)$s['user_id'],
            'by' => creator_of($r) !== null && creator_of($r) !== (int)$s['user_id'] ? ($names[creator_of($r)] ?? '') : '',
            'iCreated' => creator_of($r) !== null && creator_of($r) === (int)$s['user_id'],
        ]), array_values($rows));
        $counts = [];
        foreach (db()->query('SELECT source, COUNT(*) n FROM records WHERE deleted_at IS NULL GROUP BY source')->fetchAll() as $r) {
            $counts[$r['source']] = (int)$r['n'];
        }
        foreach ($sources as &$src) {
            $src['count'] = $counts[$src['id']] ?? 0;
        }
        $owner = is_owner($s);
        $inbox = $owner ? (int)db()->query("SELECT COUNT(*) FROM inbox WHERE state = 'nouveau'")->fetchColumn() : 0;
        send(['ok' => true, 'login' => $s['login'], 'role' => $owner ? 'owner' : 'guest', 'mode' => $owner ? 'ecriture' : $s['mode'], 'name' => $s['user_label'],
            'jarvis' => jarvis_seen(), 'sources' => $sources, 'inboxWaiting' => $inbox, 'seq' => last_seq(), 'now' => now_ms()]);
    }

    case 'GET records': {
        $s = require_web();
        $src = source_row(str_arg($_GET, 'source', 80));
        if (!$src || !can_see($s, $src['id'])) {
            fail(404, 'Source inconnue.');
        }
        $st = db()->prepare('SELECT rid, data, rev, origin, updated_at FROM records WHERE source = ? AND deleted_at IS NULL ORDER BY updated_at DESC');
        $st->execute([$src['id']]);
        $rows = array_map(fn($r) => ['id' => $r['rid'], 'data' => json_decode($r['data'], true), 'rev' => (int)$r['rev'], 'origin' => $r['origin'], 'updatedAt' => (int)$r['updated_at']], $st->fetchAll());
        send(['ok' => true, 'source' => array_merge(public_source($src), ['ops' => ops_for($s, $src)]), 'records' => $rows, 'seq' => last_seq()]);
    }

    // Modification depuis le site : enregistrée tout de suite, appliquée par Jarvis à sa prochaine lecture.
    case 'POST edit': {
        $s = require_web();
        $a = body();
        $src = source_row(str_arg($a, 'source', 80));
        $op = str_arg($a, 'op', 12);
        if (!$src || !can_see($s, $src['id'])) {
            fail(404, 'Source inconnue.');
        }
        if (!in_array($op, $WEB_OPS, true) || !in_array($op, ops_for($s, $src), true)) {
            fail(403, 'Cette modification n\'est pas permise pour cette source.');
        }
        $patch = is_array($a['data'] ?? null) ? $a['data'] : [];
        if (strlen(json_encode($patch)) > 200000) {
            fail(413, 'Contenu trop long.');
        }
        $now = now_ms();
        $pdo = db();
        $pdo->beginTransaction();
        if ($op === 'add') {
            $rid = 'w-' . token(9);
            $pdo->prepare('INSERT INTO records (source, rid, data, hash, rev, origin, updated_at) VALUES (?, ?, ?, \'\', 1, \'web\', ?)')
                ->execute([$src['id'], $rid, json_encode($patch, JSON_UNESCAPED_UNICODE), $now]);
        } else {
            $rid = str_arg($a, 'id', 120);
            $st = $pdo->prepare('SELECT data FROM records WHERE source = ? AND rid = ? AND deleted_at IS NULL FOR UPDATE');
            $st->execute([$src['id'], $rid]);
            $cur = $st->fetchColumn();
            if ($cur === false) {
                $pdo->rollBack();
                fail(404, 'Élément introuvable (supprimé entre-temps ?).');
            }
            if ($op === 'delete') {
                $pdo->prepare('UPDATE records SET deleted_at = ?, origin = \'web\', rev = rev + 1, updated_at = ? WHERE source = ? AND rid = ?')->execute([$now, $now, $src['id'], $rid]);
            } else {
                $data = json_decode($cur, true) ?: [];
                $merged = array_merge($data, $patch);
                $pdo->prepare('UPDATE records SET data = ?, origin = \'web\', rev = rev + 1, updated_at = ? WHERE source = ? AND rid = ?')
                    ->execute([json_encode($merged, JSON_UNESCAPED_UNICODE), $now, $src['id'], $rid]);
            }
        }
        $seq = log_change($src['id'], $rid, $op, 'web', $patch, $s['login']);
        $pdo->commit();
        send(['ok' => true, 'id' => $rid, 'seq' => $seq]);
    }

    case 'GET inbox': {
        require_owner_web();
        $rows = db()->query('SELECT id, kind, text, state, created_at FROM inbox ORDER BY id DESC LIMIT 50')->fetchAll();
        send(['ok' => true, 'items' => array_map(fn($r) => ['id' => (int)$r['id'], 'kind' => $r['kind'], 'text' => $r['text'], 'state' => $r['state'], 'createdAt' => (int)$r['created_at']], $rows)]);
    }

    // Demande d'une nouvelle liste (propriétaire ou invité) : un dépôt que Jarvis étudie, puis que le propriétaire valide.
    case 'POST list-request': {
        $s = require_web();
        $a = body();
        $nom = str_arg($a, 'nom', 60);
        $type = str_arg($a, 'type', 300);
        $exemples = str_arg($a, 'exemples', 500);
        $champs = $a['champs'] ?? [];
        if (is_string($champs)) {
            $champs = preg_split('/\R/u', $champs);
        }
        $champs = is_array($champs) ? $champs : [];
        $champs = array_map(fn($c) => is_string($c) ? mb_substr(trim($c), 0, 60) : '', $champs);
        $champs = array_slice(array_values(array_filter($champs, fn($c) => $c !== '')), 0, 14);
        if ($nom === '' || $type === '') {
            fail(400, 'Donne un nom et le type de liste souhaité.');
        }
        $pending = db()->prepare("SELECT COUNT(*) FROM inbox WHERE kind = 'nouvelle_liste' AND author = ? AND state IN ('nouveau', 'pris')");
        $pending->execute([$s['login']]);
        if ((int)$pending->fetchColumn() >= 5) {
            fail(409, '5 demandes en attente au plus : attends la réponse du propriétaire.');
        }
        $today = db()->prepare("SELECT COUNT(*) FROM inbox WHERE kind = 'nouvelle_liste' AND author = ? AND created_at > ?");
        $today->execute([$s['login'], now_ms() - 86400000]);
        if ((int)$today->fetchColumn() >= 10) {
            fail(429, '10 demandes par jour au plus.');
        }
        $text = json_encode(['nom' => $nom, 'type' => $type, 'champs' => $champs, 'exemples' => $exemples], JSON_UNESCAPED_UNICODE);
        db()->prepare('INSERT INTO inbox (kind, text, label, author, created_at) VALUES (?, ?, ?, ?, ?)')->execute(['nouvelle_liste', $text, $s['label'], $s['login'], now_ms()]);
        send(['ok' => true, 'id' => (int)db()->lastInsertId()]);
    }

    // Mes demandes de liste et leur avancée : nouveau (envoyée), pris (reçue par Jarvis), traite (créée), refuse.
    case 'GET list-requests': {
        $s = require_web();
        $st = db()->prepare("SELECT id, text, state, created_at FROM inbox WHERE kind = 'nouvelle_liste' AND author = ? ORDER BY id DESC LIMIT 30");
        $st->execute([$s['login']]);
        send(['ok' => true, 'items' => array_map(function ($r) {
            $j = json_decode((string)$r['text'], true);
            return ['id' => (int)$r['id'], 'nom' => is_array($j) ? (string)($j['nom'] ?? '') : '', 'type' => is_array($j) ? (string)($j['type'] ?? '') : '', 'state' => $r['state'], 'createdAt' => (int)$r['created_at']];
        }, $st->fetchAll())]);
    }

    case 'POST inbox': {
        $s = require_owner_web();
        $a = body();
        $text = str_arg($a, 'text', 5000);
        $kind = in_array($a['kind'] ?? '', ['note', 'rappel', 'demande', 'liste'], true) ? $a['kind'] : 'note';
        if ($text === '') {
            fail(400, 'Texte vide.');
        }
        db()->prepare('INSERT INTO inbox (kind, text, label, author, created_at) VALUES (?, ?, ?, ?, ?)')->execute([$kind, $text, $s['label'], $s['login'], now_ms()]);
        send(['ok' => true, 'id' => (int)db()->lastInsertId()]);
    }

    case 'GET sessions': {
        $s = web_session();
        if (!$s) {
            require_device();
        }
        // Un invité ne voit que ses propres connexions.
        $st = db()->prepare('SELECT s.id, s.label, s.ip, s.created_at, s.last_seen, s.expires_at, u.login FROM sessions s JOIN users u ON u.id = s.user_id
            WHERE ? = 1 OR s.user_id = ? ORDER BY s.last_seen DESC');
        $st->execute([(!$s || is_owner($s)) ? 1 : 0, $s ? (int)$s['user_id'] : 0]);
        $rows = $st->fetchAll();
        $devices = db()->query('SELECT id, name, version, created_at, last_seen FROM devices ORDER BY last_seen DESC')->fetchAll();
        send(['ok' => true, 'current' => $s ? (int)$s['id'] : null, 'sessions' => $rows, 'devices' => $devices]);
    }

    case 'POST revoke': {
        $s = web_session();
        if ($s) {
            require_web();
        } else {
            require_device();
        }
        $a = body();
        if ($s && !is_owner($s)) {
            db()->prepare('DELETE FROM sessions WHERE id = ? AND user_id = ?')->execute([(int)($a['id'] ?? 0), (int)$s['user_id']]);
        } else {
            db()->prepare('DELETE FROM sessions WHERE id = ?')->execute([(int)($a['id'] ?? 0)]);
        }
        send(['ok' => true]);
    }

    // --- Listes des invités ---------------------------------------------------------------------

    case 'POST list-create': {
        $s = require_web();
        if (is_owner($s)) {
            fail(403, 'Tes listes se créent dans Jarvis.');
        }
        $a = body();
        $title = str_arg($a, 'title', 120);
        if ($title === '') {
            fail(400, 'Donne un nom à la liste.');
        }
        $n = db()->prepare('SELECT COUNT(*) FROM sources WHERE owner_user = ?');
        $n->execute([$s['user_id']]);
        if ((int)$n->fetchColumn() >= MAX_GUEST_LISTS) {
            fail(409, MAX_GUEST_LISTS . ' listes au plus.');
        }
        $id = GUEST_LIST_PREFIX . (int)$s['user_id'] . '-' . strtolower(preg_replace('/[^a-z0-9]/i', '', token(9)));
        db()->prepare('INSERT INTO sources (id, label, grp, kind, ops, meta, position, updated_at, owner_user) VALUES (?, ?, ?, \'liste\', ?, NULL, 1000, ?, ?)')
            ->execute([$id, $title, 'Mes listes', GUEST_LIST_OPS, now_ms(), $s['user_id']]);
        log_change($id, '', 'list', 'web', ['title' => $title], $s['login']);
        send(['ok' => true, 'id' => $id]);
    }

    case 'POST list-rename': {
        $s = require_web();
        $a = body();
        $src = guest_list_for($s, str_arg($a, 'id', 80));
        $title = str_arg($a, 'title', 120);
        if ($title === '') {
            fail(400, 'Donne un nom à la liste.');
        }
        db()->prepare('UPDATE sources SET label = ?, updated_at = ? WHERE id = ?')->execute([$title, now_ms(), $src['id']]);
        log_change($src['id'], '', 'list', 'web', ['title' => $title], $s['login']);
        send(['ok' => true]);
    }

    case 'POST list-delete': {
        $s = require_web();
        $a = body();
        $src = guest_list_for($s, str_arg($a, 'id', 80));
        drop_guest_list($src['id']);
        send(['ok' => true]);
    }

    // --- Invitations (sans session) -------------------------------------------------------------

    case 'POST invite-info': {
        $a = body();
        check_attempts();
        $u = invited_user(str_arg($a, 'login', 80), (string)($a['token'] ?? ''));
        if (!$u) {
            note_attempt();
            fail(404, 'Invitation inconnue ou expirée : demande un nouveau lien.');
        }
        send(['ok' => true, 'login' => $u['login'], 'name' => $u['label'], 'active' => $u['password_hash'] !== null]);
    }

    // Première visite (ou nouveau lien) : l'invité choisit son mot de passe, haché en base.
    case 'POST activate': {
        $a = body();
        check_attempts();
        $u = invited_user(str_arg($a, 'login', 80), (string)($a['token'] ?? ''));
        if (!$u) {
            note_attempt();
            fail(404, 'Invitation inconnue ou expirée : demande un nouveau lien.');
        }
        $password = (string)($a['password'] ?? '');
        if (mb_strlen($password) < 10) {
            fail(400, 'Mot de passe de 10 caractères au moins.');
        }
        db()->prepare('UPDATE users SET password_hash = ?, invite_hash = NULL, invite_expires = NULL, updated_at = ? WHERE id = ?')->execute([hash_password($password), now_ms(), $u['id']]);
        db()->prepare('DELETE FROM sessions WHERE user_id = ?')->execute([$u['id']]);
        open_session((int)$u['id'], str_arg($a, 'label', 160) ?: substr((string)($_SERVER['HTTP_USER_AGENT'] ?? ''), 0, 160));
        send(['ok' => true]);
    }

    // Un invité change son mot de passe (celui du propriétaire se règle dans l'administration de Jarvis).
    case 'POST password': {
        $s = require_web();
        if (is_owner($s)) {
            fail(403, 'Le mot de passe du propriétaire se change dans l\'administration de Jarvis.');
        }
        $a = body();
        check_attempts();
        if (!check_credentials($s['login'], (string)($a['old'] ?? ''))) {
            note_attempt();
            fail(401, 'Mot de passe actuel incorrect.');
        }
        if (mb_strlen((string)($a['new'] ?? '')) < 10) {
            fail(400, 'Nouveau mot de passe de 10 caractères au moins.');
        }
        db()->prepare('UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?')->execute([hash_password((string)$a['new']), now_ms(), $s['user_id']]);
        db()->prepare('DELETE FROM sessions WHERE user_id = ? AND id <> ?')->execute([$s['user_id'], $s['id']]);
        send(['ok' => true]);
    }

    // --- Jarvis -------------------------------------------------------------------------------

    // Relier un Jarvis : identifiant et mot de passe du site. Premier appel sur un site vide : le code
    // d'installation (écrit par deploy.ps1 chez Jarvis) crée le compte.
    case 'POST pair': {
        $a = body();
        check_attempts();
        $login = str_arg($a, 'login', 80);
        $password = (string)($a['password'] ?? '');
        $users = (int)db()->query("SELECT COUNT(*) FROM users WHERE role = 'owner'")->fetchColumn();
        if ($users === 0) {
            if (!hash_equals((string)cfg()['install_token'], (string)($a['install'] ?? ''))) {
                note_attempt();
                fail(403, 'Site vierge : code d\'installation absent ou faux.');
            }
            if (!preg_match('/^[\w.@-]{3,80}$/u', $login) || mb_strlen($password) < 10) {
                fail(400, 'Identifiant (3 caractères, lettres, chiffres, . @ - _) et mot de passe de 10 caractères au moins.');
            }
            $now = now_ms();
            db()->prepare('INSERT INTO users (login, password_hash, created_at, updated_at) VALUES (?, ?, ?, ?)')->execute([$login, hash_password($password), $now, $now]);
        } elseif (!($u = check_credentials($login, $password)) || $u['role'] !== 'owner') {
            note_attempt();
            fail(401, 'Identifiant ou mot de passe incorrect.');
        }
        $key = token(32);
        $now = now_ms();
        db()->prepare('INSERT INTO devices (name, key_hash, created_at, last_seen) VALUES (?, ?, ?, ?)')->execute([str_arg($a, 'device', 80) ?: 'Jarvis', sha($key), $now, $now]);
        send(['ok' => true, 'key' => $key, 'created' => $users === 0]);
    }

    case 'POST account': {
        require_device();
        $a = body();
        $login = str_arg($a, 'login', 80);
        $password = (string)($a['password'] ?? '');
        if (!preg_match('/^[\w.@-]{3,80}$/u', $login) || mb_strlen($password) < 10) {
            fail(400, 'Identifiant (3 caractères, lettres, chiffres, . @ - _) et mot de passe de 10 caractères au moins.');
        }
        $taken = db()->prepare("SELECT COUNT(*) FROM users WHERE login = ? AND role <> 'owner'");
        $taken->execute([$login]);
        if ((int)$taken->fetchColumn() > 0) {
            fail(409, 'Cet identifiant est déjà celui d\'un partage.');
        }
        db()->prepare("UPDATE users SET login = ?, password_hash = ?, updated_at = ? WHERE role = 'owner'")->execute([$login, hash_password($password), now_ms()]);
        if (!empty($a['revokeSessions'])) {
            db()->exec("DELETE s FROM sessions s JOIN users u ON u.id = s.user_id WHERE u.role = 'owner'");
        }
        send(['ok' => true]);
    }

    // Version déployée (écrite par deploy.ps1) : Jarvis compare les empreintes à son dossier web-nomade.
    case 'GET version': {
        if (!web_session()) {
            require_device();
        }
        $v = json_decode((string)@file_get_contents(__DIR__ . '/../private/version.json'), true);
        $schema = [];
        try {
            $schema = db()->query('SELECT name FROM schema_migrations ORDER BY name')->fetchAll(PDO::FETCH_COLUMN);
        } catch (Throwable $e) {
        }
        send(['ok' => true, 'version' => $v['version'] ?? null, 'commit' => $v['commit'] ?? null, 'modifie' => (bool)($v['modifie'] ?? false),
            'deployedAt' => (int)($v['deployedAt'] ?? 0), 'files' => $v['files'] ?? (object)[], 'migrations' => $schema, 'php' => PHP_VERSION]);
    }

    case 'GET ping': {
        require_device();
        send(['ok' => true, 'seq' => last_seq(), 'now' => now_ms()]);
    }

    // Envoi de Jarvis : sources décrites, enregistrements ajoutés ou modifiés, suppressions.
    case 'POST push': {
        require_device();
        $a = body();
        $now = now_ms();
        $pdo = db();
        $pdo->beginTransaction();
        foreach (($a['sources'] ?? []) as $i => $s) {
            if (!is_array($s) || !preg_match('/^[\w:.-]{1,80}$/', (string)($s['id'] ?? ''))) {
                continue;
            }
            $ops = implode(',', array_intersect($WEB_OPS, is_array($s['ops'] ?? null) ? $s['ops'] : []));
            $pdo->prepare('INSERT INTO sources (id, label, grp, kind, ops, meta, position, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON DUPLICATE KEY UPDATE label = VALUES(label), grp = VALUES(grp), kind = VALUES(kind), ops = VALUES(ops), meta = VALUES(meta), position = VALUES(position), updated_at = VALUES(updated_at)')
                ->execute([$s['id'], str_arg($s, 'label', 160), str_arg($s, 'group', 60), str_arg($s, 'kind', 30), $ops, json_encode($s['meta'] ?? null, JSON_UNESCAPED_UNICODE), (int)($s['position'] ?? $i), $now]);
        }
        $up = $pdo->prepare('INSERT INTO records (source, rid, data, hash, rev, origin, updated_at, deleted_at) VALUES (?, ?, ?, ?, 1, \'pc\', ?, NULL)
            ON DUPLICATE KEY UPDATE data = VALUES(data), hash = VALUES(hash), rev = rev + 1, origin = \'pc\', updated_at = VALUES(updated_at), deleted_at = NULL');
        $n = 0;
        foreach (($a['upserts'] ?? []) as $r) {
            $src = is_array($r) ? source_row((string)($r['source'] ?? '')) : null;
            if (!$src || $src['owner_user'] !== null) {
                continue; // les listes des invités ne sont jamais écrites par Jarvis
            }
            $rid = mb_substr((string)($r['id'] ?? ''), 0, 120);
            $up->execute([$r['source'], $rid, json_encode($r['data'] ?? null, JSON_UNESCAPED_UNICODE), substr((string)($r['hash'] ?? ''), 0, 64), $now]);
            log_change($r['source'], $rid, 'upsert', 'pc');
            $n++;
        }
        $del = $pdo->prepare('UPDATE records SET deleted_at = ?, origin = \'pc\', rev = rev + 1, updated_at = ? WHERE source = ? AND rid = ? AND deleted_at IS NULL');
        foreach (($a['deletes'] ?? []) as $r) {
            $src = is_array($r) ? source_row((string)($r['source'] ?? '')) : null;
            if (!$src || $src['owner_user'] !== null) {
                continue;
            }
            $del->execute([$now, $now, (string)($r['source'] ?? ''), (string)($r['id'] ?? '')]);
            if ($del->rowCount() > 0) {
                log_change((string)$r['source'], (string)$r['id'], 'delete', 'pc');
            }
            $n++;
        }
        $pdo->commit();
        send(['ok' => true, 'applied' => $n, 'seq' => last_seq()]);
    }

    // Lecture de Jarvis : modifications faites sur le site après `since`, et la boîte de dépôt.
    case 'GET pull': {
        require_device();
        $since = max(0, (int)($_GET['since'] ?? 0));
        $st = db()->prepare('SELECT c.seq, c.source, c.rid, c.op, c.patch, c.at, c.author, r.data FROM changes c LEFT JOIN records r ON r.source = c.source AND r.rid = c.rid
            WHERE c.origin = \'web\' AND c.seq > ? ORDER BY c.seq LIMIT 500');
        $st->execute([$since]);
        $changes = array_map(fn($r) => [
            'seq' => (int)$r['seq'], 'source' => $r['source'], 'id' => $r['rid'], 'op' => $r['op'],
            'patch' => $r['patch'] ? json_decode($r['patch'], true) : null, 'data' => $r['data'] ? json_decode($r['data'], true) : null, 'at' => (int)$r['at'], 'author' => $r['author'],
        ], $st->fetchAll());
        $inbox = db()->query("SELECT i.id, i.kind, i.text, i.label, i.author, i.created_at, u.label AS author_name, u.relation AS author_relation, u.role AS author_role
            FROM inbox i LEFT JOIN users u ON u.login = i.author WHERE i.state = 'nouveau' ORDER BY i.id LIMIT 100")->fetchAll();
        send(['ok' => true, 'changes' => $changes, 'inbox' => array_map(fn($r) => ['id' => (int)$r['id'], 'kind' => $r['kind'], 'text' => $r['text'], 'label' => $r['label'], 'author' => $r['author'], 'authorName' => $r['author_name'] ?: $r['author'], 'authorRelation' => $r['author_relation'], 'authorRole' => $r['author_role'], 'createdAt' => (int)$r['created_at']], $inbox), 'seq' => last_seq(), 'more' => count($changes) === 500]);
    }

    case 'POST inbox-ack': {
        require_device();
        $a = body();
        $state = in_array($a['state'] ?? '', ['pris', 'traite', 'refuse'], true) ? $a['state'] : 'pris';
        $st = db()->prepare('UPDATE inbox SET state = ?, taken_at = ? WHERE id = ?');
        foreach (($a['ids'] ?? []) as $id) {
            $st->execute([$state, now_ms(), (int)$id]);
        }
        send(['ok' => true]);
    }

    // Images des fiches : nommées par l'empreinte de leur contenu, gardées dans private/files (inaccessible
    // directement), servies aux seules sessions du site.
    case 'POST has': {
        require_device();
        $a = body();
        $missing = [];
        foreach (array_slice(is_array($a['names'] ?? null) ? $a['names'] : [], 0, 2000) as $n) {
            if (is_string($n) && valid_file_name($n) && !is_file(files_dir() . '/' . $n)) {
                $missing[] = $n;
            }
        }
        send(['ok' => true, 'missing' => $missing]);
    }

    case 'POST upload': {
        require_device();
        $a = body();
        $name = (string)($a['name'] ?? '');
        $bytes = base64_decode((string)($a['data'] ?? ''), true);
        if (!valid_file_name($name) || $bytes === false || $bytes === '' || strlen($bytes) > MAX_FILE) {
            fail(400, 'Image refusée.');
        }
        if (!hash_equals(substr($name, 0, 64), hash('sha256', $bytes))) {
            fail(400, 'Empreinte de l\'image incorrecte.');
        }
        $dir = files_dir();
        if (!is_dir($dir) && !mkdir($dir, 0750, true)) {
            fail(500, 'Dossier des images impossible à créer.');
        }
        $tmp = $dir . '/.' . $name . '.tmp';
        if (file_put_contents($tmp, $bytes) === false || !rename($tmp, $dir . '/' . $name)) {
            fail(500, 'Image non enregistrée.');
        }
        send(['ok' => true]);
    }

    // Images plus utilisées par aucun enregistrement : effacées.
    case 'POST gc': {
        require_device();
        $used = [];
        foreach (db()->query("SELECT data FROM records WHERE deleted_at IS NULL AND data LIKE '%\"_img\"%'") as $r) {
            $d = json_decode($r['data'], true);
            if (is_array($d) && is_string($d['_img'] ?? null)) {
                $used[$d['_img']] = true;
            }
        }
        $removed = 0;
        foreach (glob(files_dir() . '/*') ?: [] as $f) {
            if (!isset($used[basename($f)]) && @unlink($f)) {
                $removed++;
            }
        }
        send(['ok' => true, 'removed' => $removed]);
    }

    case 'GET file': {
        $s = require_web();
        $name = (string)($_GET['n'] ?? '');
        $file = files_dir() . '/' . $name;
        if (!valid_file_name($name) || !is_file($file)) {
            fail(404, 'Image introuvable.');
        }
        // Invité : seulement les images des fiches qui lui sont partagées.
        if (!is_owner($s)) {
            $st = db()->prepare('SELECT 1 FROM records r JOIN user_sources us ON us.source = r.source WHERE us.user_id = ? AND r.deleted_at IS NULL AND r.data LIKE ? LIMIT 1');
            $st->execute([$s['user_id'], '%"_img":"' . $name . '"%']);
            if (!$st->fetchColumn()) {
                fail(404, 'Image introuvable.');
            }
        }
        $types = ['jpg' => 'image/jpeg', 'png' => 'image/png', 'gif' => 'image/gif', 'webp' => 'image/webp'];
        header('Content-Type: ' . $types[pathinfo($name, PATHINFO_EXTENSION)]);
        header('Content-Length: ' . filesize($file));
        header('Cache-Control: private, max-age=31536000, immutable');
        header('X-Content-Type-Options: nosniff');
        readfile($file);
        exit;
    }

    // --- Partages, gérés depuis l'administration de Jarvis ----------------------------------------

    case 'GET guests': {
        require_device();
        $rows = db()->query("SELECT * FROM users WHERE role = 'guest' ORDER BY label, login")->fetchAll();
        send(['ok' => true, 'guests' => array_map('guest_row', $rows)]);
    }

    // Création (avec jeton d'invitation) ou modification d'un partage : mode et sources visibles.
    case 'POST guest-save': {
        require_device();
        $a = body();
        $id = (int)($a['id'] ?? 0);
        $login = str_arg($a, 'login', 80);
        $email = str_arg($a, 'email', 190);
        $mode = ($a['mode'] ?? '') === 'ecriture' ? 'ecriture' : 'lecture';
        $relation = in_array($a['relation'] ?? '', RELATIONS, true) ? $a['relation'] : 'ami';
        $sources = array_values(array_filter(is_array($a['sources'] ?? null) ? $a['sources'] : [], fn($x) => is_string($x) && source_row($x)));
        if (!valid_login_name($login)) {
            fail(400, 'Identifiant de 3 caractères au moins (lettres, chiffres, . _ @ -).');
        }
        if ($email !== '' && !filter_var($email, FILTER_VALIDATE_EMAIL)) {
            fail(400, 'Adresse mail invalide.');
        }
        $taken = db()->prepare('SELECT id FROM users WHERE login = ? AND id <> ?');
        $taken->execute([$login, $id]);
        if ($taken->fetchColumn()) {
            fail(409, 'Cet identifiant est déjà pris.');
        }
        $pdo = db();
        $pdo->beginTransaction();
        $now = now_ms();
        if ($id > 0) {
            $chk = $pdo->prepare("SELECT COUNT(*) FROM users WHERE id = ? AND role = 'guest'");
            $chk->execute([$id]);
            if (!(int)$chk->fetchColumn()) {
                $pdo->rollBack();
                fail(404, 'Partage introuvable.');
            }
            $pdo->prepare("UPDATE users SET login = ?, email = ?, label = ?, mode = ?, relation = ?, updated_at = ? WHERE id = ? AND role = 'guest'")
                ->execute([$login, $email, str_arg($a, 'label', 120), $mode, $relation, $now, $id]);
        } else {
            $pdo->prepare("INSERT INTO users (login, password_hash, role, email, label, mode, relation, created_at, updated_at) VALUES (?, NULL, 'guest', ?, ?, ?, ?, ?, ?)")
                ->execute([$login, $email, str_arg($a, 'label', 120), $mode, $relation, $now, $now]);
            $id = (int)$pdo->lastInsertId();
        }
        // Les écritures accordées (listes créées à la demande de l'invité) survivent à l'enregistrement du partage.
        $granted = [];
        $byCreator = [];
        try {
            $g = $pdo->prepare('SELECT source FROM user_sources WHERE user_id = ? AND write_ok = 1');
            $g->execute([$id]);
            $granted = $g->fetchAll(PDO::FETCH_COLUMN);
            // Accès réglés par le créateur d'une liste (partage d'un invité) : l'enregistrement du partage ne les efface pas.
            $c = $pdo->prepare('SELECT us.source, us.write_ok FROM user_sources us JOIN sources s ON s.id = us.source WHERE us.user_id = ? AND s.creator_user IS NOT NULL');
            $c->execute([$id]);
            $byCreator = $c->fetchAll();
        } catch (PDOException $e) {
            // migration 004 ou 005 pas encore appliquée
        }
        $pdo->prepare('DELETE FROM user_sources WHERE user_id = ?')->execute([$id]);
        $ins = $pdo->prepare('INSERT INTO user_sources (user_id, source, write_ok) VALUES (?, ?, ?)');
        $kept = [];
        foreach (array_unique($sources) as $src) {
            $ins->execute([$id, $src, in_array($src, $granted, true) ? 1 : 0]);
            $kept[$src] = true;
        }
        foreach ($byCreator as $row) {
            if (!isset($kept[$row['source']])) {
                $ins->execute([$id, $row['source'], (int)$row['write_ok']]);
            }
        }
        $token = empty($a['invite']) ? null : new_invite($id);
        $pdo->commit();
        $st = db()->prepare('SELECT * FROM users WHERE id = ?');
        $st->execute([$id]);
        send(['ok' => true, 'guest' => guest_row($st->fetch()), 'token' => $token]);
    }

    // Liste créée par Jarvis à la demande d'un invité : il peut la modifier (le propriétaire aussi), aucun autre invité ne la voit.
    case 'POST guest-grant': {
        require_device();
        $a = body();
        $login = str_arg($a, 'login', 80);
        $source = str_arg($a, 'source', 80);
        $st = db()->prepare("SELECT id FROM users WHERE login = ? AND role = 'guest'");
        $st->execute([$login]);
        $uid = (int)$st->fetchColumn();
        if (!$uid) {
            fail(404, 'Invité introuvable.');
        }
        if (!source_row($source)) {
            fail(404, 'Source inconnue : synchroniser d\'abord.');
        }
        db()->prepare('INSERT INTO user_sources (user_id, source, write_ok) VALUES (?, ?, 1) ON DUPLICATE KEY UPDATE write_ok = 1')->execute([$uid, $source]);
        // Le demandeur devient le créateur de la liste : lui seul en règle le partage (le propriétaire la voit toujours).
        db()->prepare('UPDATE sources SET creator_user = ? WHERE id = ? AND creator_user IS NULL AND owner_user IS NULL')->execute([$uid, $source]);
        send(['ok' => true]);
    }

    // Partage d'une liste créée par un invité : lui seul (son créateur) décide, pour chaque autre invité, aucun accès / lecture /
    // lecture et écriture. Le propriétaire du site la voit toujours.
    case 'GET list-shares': {
        $s = require_web();
        $src = shareable_list($s, str_arg($_GET, 'id', 80));
        $st = db()->prepare('SELECT source, write_ok FROM user_sources WHERE user_id = ? AND source = ?');
        $out = [];
        foreach (db()->query("SELECT id, label, login FROM users WHERE role = 'guest' AND password_hash IS NOT NULL ORDER BY label, login")->fetchAll() as $u) {
            if ((int)$u['id'] === creator_of($src)) {
                continue;
            }
            $st->execute([$u['id'], $src['id']]);
            $row = $st->fetch();
            $out[] = ['user' => (int)$u['id'], 'name' => $u['label'] ?: $u['login'], 'mode' => !$row ? 'none' : ((int)$row['write_ok'] === 1 ? 'ecriture' : 'lecture')];
        }
        $owner = db()->query("SELECT label, login FROM users WHERE role = 'owner' LIMIT 1")->fetch();
        send(['ok' => true, 'guests' => $out, 'owner' => $owner ? ($owner['label'] ?: $owner['login']) : '']);
    }

    case 'POST list-share': {
        $s = require_web();
        $a = body();
        $src = shareable_list($s, str_arg($a, 'id', 80));
        $pdo = db();
        $pdo->beginTransaction();
        $del = $pdo->prepare('DELETE FROM user_sources WHERE user_id = ? AND source = ?');
        $up = $pdo->prepare('INSERT INTO user_sources (user_id, source, write_ok) VALUES (?, ?, ?) ON DUPLICATE KEY UPDATE write_ok = VALUES(write_ok)');
        $guest = $pdo->prepare("SELECT COUNT(*) FROM users WHERE id = ? AND role = 'guest'");
        foreach ((is_array($a['shares'] ?? null) ? $a['shares'] : []) as $x) {
            $uid = (int)($x['user'] ?? 0);
            $guest->execute([$uid]);
            if (!$uid || $uid === creator_of($src) || !(int)$guest->fetchColumn()) {
                continue;
            }
            $mode = $x['mode'] ?? 'none';
            if ($mode === 'lecture' || $mode === 'ecriture') {
                $up->execute([$uid, $src['id'], $mode === 'ecriture' ? 1 : 0]);
            } else {
                $del->execute([$uid, $src['id']]);
            }
        }
        $pdo->commit();
        send(['ok' => true]);
    }

    // Listes créées par les invités, avec leur contenu : Jarvis les range dans sa mémoire (famille, amis…).
    case 'GET friends': {
        require_device();
        $out = [];
        $users = db()->query("SELECT id, login, label, relation FROM users WHERE role = 'guest'")->fetchAll();
        $recs = db()->prepare('SELECT rid, data, updated_at FROM records WHERE source = ? AND deleted_at IS NULL ORDER BY updated_at');
        foreach ($users as $u) {
            $st = db()->prepare('SELECT id, label, updated_at FROM sources WHERE owner_user = ? ORDER BY label');
            $st->execute([$u['id']]);
            $lists = [];
            foreach ($st->fetchAll() as $l) {
                $recs->execute([$l['id']]);
                $lists[] = ['id' => $l['id'], 'title' => $l['label'], 'updatedAt' => (int)$l['updated_at'],
                    'items' => array_map(fn($r) => ['id' => $r['rid'], 'data' => json_decode($r['data'], true)], $recs->fetchAll())];
            }
            $out[] = ['id' => (int)$u['id'], 'login' => $u['login'], 'name' => $u['label'] ?: $u['login'], 'relation' => $u['relation'], 'lists' => $lists];
        }
        send(['ok' => true, 'friends' => $out]);
    }

    // Nouveau lien d'invitation (premier mail perdu, mot de passe oublié) : l'ancien lien ne marche plus.
    case 'POST guest-invite': {
        require_device();
        $a = body();
        $st = db()->prepare("SELECT * FROM users WHERE id = ? AND role = 'guest'");
        $st->execute([(int)($a['id'] ?? 0)]);
        $u = $st->fetch();
        if (!$u) {
            fail(404, 'Partage introuvable.');
        }
        send(['ok' => true, 'token' => new_invite((int)$u['id']), 'guest' => guest_row($u)]);
    }

    case 'POST guest-delete': {
        require_device();
        $a = body();
        $id = (int)($a['id'] ?? 0);
        $pdo = db();
        $pdo->beginTransaction();
        $lists = $pdo->prepare('SELECT id FROM sources WHERE owner_user = ?');
        $lists->execute([$id]);
        foreach ($lists->fetchAll(PDO::FETCH_COLUMN) as $lid) {
            drop_guest_list($lid);
        }
        foreach (['DELETE FROM sessions WHERE user_id = ?', 'DELETE FROM user_sources WHERE user_id = ?', "DELETE FROM users WHERE id = ? AND role = 'guest'"] as $sql) {
            $pdo->prepare($sql)->execute([$id]);
        }
        $pdo->commit();
        send(['ok' => true]);
    }

    // Source décochée dans l'administration de Jarvis : tout ce qui la concerne est effacé.
    case 'POST drop': {
        require_device();
        $a = body();
        $id = str_arg($a, 'source', 80);
        $src = source_row($id);
        if ($src && $src['owner_user'] !== null) {
            fail(403, 'Une liste d\'invité ne se retire pas depuis Jarvis.');
        }
        $pdo = db();
        $pdo->beginTransaction();
        foreach (['DELETE FROM records WHERE source = ?', 'DELETE FROM changes WHERE source = ?', 'DELETE FROM user_sources WHERE source = ?', 'DELETE FROM sources WHERE id = ?'] as $sql) {
            $pdo->prepare($sql)->execute([$id]);
        }
        $pdo->commit();
        send(['ok' => true]);
    }

    default:
        fail(404, 'Route inconnue.');
}
