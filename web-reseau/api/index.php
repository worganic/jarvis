<?php
// Jarvis Réseau — API des Jarvis entre eux (crates/jarvis-core/src/orch/reseau/). Route : api/?r=<nom>, JSON.
//
// Un compte = un mail vérifié par code + une clé publique X25519 (la clé privée ne quitte jamais le PC). Une demande
// d'ami se fait par mail ; acceptée, les deux Jarvis s'échangent des envois chiffrés de bout en bout (messages, listes,
// fichiers) par la boîte aux lettres res_items : le serveur ne voit que l'expéditeur, le destinataire, le type et la
// taille. La présence (en ligne, absent…) n'est visible que des amis à qui elle est partagée.

declare(strict_types=1);
require __DIR__ . '/lib.php';

header('X-Content-Type-Options: nosniff');
header('Referrer-Policy: no-referrer');

$route = preg_replace('/[^a-z-]/', '', (string)($_GET['r'] ?? ''));
$method = $_SERVER['REQUEST_METHOD'];

function public_account(array $a): array {
    return ['id' => (int)$a['id'], 'nom' => $a['name'], 'email' => $a['email'], 'cle' => $a['public_key']];
}

switch ("$method $route") {

    case 'GET version': {
        send(['ok' => true, 'service' => 'jarvis-reseau', 'api' => 1]);
    }

    // --- Inscription : code reçu par mail, puis clé d'appareil ------------------------------------------------------

    case 'POST inscription': {
        $a = body();
        $email = strtolower(str_arg($a, 'email', 190));
        $name = str_arg($a, 'nom', 80);
        $key = str_arg($a, 'cle', 64);
        if (!valid_email($email) || $name === '' || !valid_key($key)) {
            fail(400, 'Mail, nom ou clé invalide.');
        }
        rate('inscription-ip:' . client_ip(), 6, 3600 * 1000, 'Trop de demandes : réessaie dans une heure.');
        rate('inscription-mail:' . $email, 3, 3600 * 1000, 'Trop de codes demandés pour ce mail : réessaie dans une heure.');
        $code = str_pad((string)random_int(0, 999999), 6, '0', STR_PAD_LEFT);
        db()->prepare('REPLACE INTO res_email_codes (email, code_hash, name, public_key, expires_at, tries) VALUES (?, ?, ?, ?, ?, 0)')
            ->execute([$email, sha($email . ':' . $code), $name, $key, now_ms() + CODE_MINUTES * 60 * 1000]);
        // Essais locaux seulement (configuration écrite à la main, jamais par deploy.ps1) : code rendu au lieu d'être envoyé.
        if (!empty(cfg()['test_codes'])) {
            send(['ok' => true, 'minutes' => CODE_MINUTES, 'code' => $code]);
        }
        $sent = mail_to($email, "Ton code Jarvis : $code",
            "Bonjour $name,\n\nVoici le code à donner à Jarvis pour relier ton adresse : $code\n\nIl est valable " . CODE_MINUTES . " minutes. Si tu n'as rien demandé, ignore ce message.\n\nJarvis");
        if (!$sent) {
            fail(503, 'Le mail n\'a pas pu partir : réessaie plus tard.');
        }
        send(['ok' => true, 'minutes' => CODE_MINUTES]);
    }

    case 'POST verification': {
        $a = body();
        $email = strtolower(str_arg($a, 'email', 190));
        $code = preg_replace('/\D/', '', str_arg($a, 'code', 12));
        $st = db()->prepare('SELECT * FROM res_email_codes WHERE email = ?');
        $st->execute([$email]);
        $c = $st->fetch();
        if (!$c || (int)$c['expires_at'] < now_ms() || (int)$c['tries'] >= 5) {
            fail(400, 'Code expiré : demandes-en un nouveau.');
        }
        if (!hash_equals($c['code_hash'], sha($email . ':' . $code))) {
            db()->prepare('UPDATE res_email_codes SET tries = tries + 1 WHERE email = ?')->execute([$email]);
            fail(400, 'Code incorrect.');
        }
        db()->prepare('DELETE FROM res_email_codes WHERE email = ?')->execute([$email]);
        $acc = account_by_email($email);
        if ($acc) {
            // Nouveau PC ou réinstallation : la clé publique suit l'appareil qui vient de prouver le mail.
            db()->prepare('UPDATE res_accounts SET name = ?, public_key = ? WHERE id = ?')->execute([$c['name'], $c['public_key'], $acc['id']]);
            $id = (int)$acc['id'];
        } else {
            db()->prepare('INSERT INTO res_accounts (email, name, public_key, created_at) VALUES (?, ?, ?, ?)')->execute([$email, $c['name'], $c['public_key'], now_ms()]);
            $id = (int)db()->lastInsertId();
        }
        $device = token(32);
        db()->prepare('INSERT INTO res_devices (account_id, key_hash, label, created_at) VALUES (?, ?, ?, ?)')
            ->execute([$id, sha($device), str_arg($a, 'appareil', 120), now_ms()]);
        // Invitations reçues avant l'inscription : elles deviennent des demandes d'ami.
        $inv = db()->prepare('SELECT requester_id FROM res_invites WHERE email = ?');
        $inv->execute([$email]);
        foreach ($inv->fetchAll(PDO::FETCH_COLUMN) as $rid) {
            if ((int)$rid !== $id && !friendship((int)$rid, $id)) {
                db()->prepare('INSERT INTO res_friendships (requester_id, target_id, status, created_at, updated_at) VALUES (?, ?, \'pending\', ?, ?)')
                    ->execute([(int)$rid, $id, now_ms(), now_ms()]);
            }
        }
        db()->prepare('DELETE FROM res_invites WHERE email = ?')->execute([$email]);
        $acc = account_by_email($email);
        send(['ok' => true, 'cleAppareil' => $device, 'compte' => public_account($acc)]);
    }

    // --- Compte -----------------------------------------------------------------------------------------------------

    case 'GET moi': {
        $me = require_account();
        send(['ok' => true, 'compte' => public_account($me)]);
    }

    case 'POST profil': {
        $me = require_account();
        $a = body();
        $name = str_arg($a, 'nom', 80);
        if ($name !== '') {
            db()->prepare('UPDATE res_accounts SET name = ? WHERE id = ?')->execute([$name, $me['id']]);
        }
        send(['ok' => true, 'compte' => public_account(account_by_email($me['email']))]);
    }

    // Droit à l'effacement : compte, appareils, amitiés, partages, envois et présence.
    case 'POST compte-supprimer': {
        $me = require_account();
        $id = (int)$me['id'];
        $pdo = db();
        $pdo->prepare('DELETE FROM res_items WHERE sender_id = ? OR recipient_id = ?')->execute([$id, $id]);
        $pdo->prepare('DELETE FROM res_shares WHERE owner_id = ? OR friend_id = ?')->execute([$id, $id]);
        $pdo->prepare('DELETE FROM res_friendships WHERE requester_id = ? OR target_id = ?')->execute([$id, $id]);
        $pdo->prepare('DELETE FROM res_invites WHERE requester_id = ? OR email = ?')->execute([$id, $me['email']]);
        $pdo->prepare('DELETE FROM res_presence WHERE account_id = ?')->execute([$id]);
        $pdo->prepare('DELETE FROM res_devices WHERE account_id = ?')->execute([$id]);
        $pdo->prepare('DELETE FROM res_accounts WHERE id = ?')->execute([$id]);
        send(['ok' => true]);
    }

    // --- Amis -------------------------------------------------------------------------------------------------------

    case 'POST ami-demander': {
        $me = require_account();
        $a = body();
        $email = strtolower(str_arg($a, 'email', 190));
        if (!valid_email($email)) {
            fail(400, 'Mail invalide.');
        }
        if ($email === $me['email']) {
            fail(400, 'C\'est ton propre mail.');
        }
        rate('ami:' . $me['id'], 30, 86400 * 1000, 'Trop de demandes d\'ami aujourd\'hui.');
        $other = account_by_email($email);
        if (!$other) {
            db()->prepare('INSERT IGNORE INTO res_invites (requester_id, email, created_at) VALUES (?, ?, ?)')->execute([$me['id'], $email, now_ms()]);
            $install = (string)(cfg()['install_command'] ?? '');
            $service = 'https://' . ($_SERVER['HTTP_HOST'] ?? '') . rtrim(dirname(dirname((string)($_SERVER['SCRIPT_NAME'] ?? '/api/index.php'))), '/\\');
            mail_to($email, $me['name'] . ' t\'invite sur Jarvis',
                $me['name'] . " (" . $me['email'] . ") aimerait échanger avec toi par Jarvis, son assistant vocal.\n\n"
                . ($install !== '' ? "Pour installer Jarvis sur ton PC Windows, colle ceci dans PowerShell :\n$install\n\n" : '')
                . "Inscris-toi ensuite avec ce mail (Jarvis › Amis), adresse du service : $service\nLa demande d'ami t'attendra.\n\nJarvis");
            send(['ok' => true, 'statut' => 'invite']);
        }
        $f = friendship((int)$me['id'], (int)$other['id']);
        if ($f) {
            if ($f['status'] === 'accepted') {
                send(['ok' => true, 'statut' => 'deja-amis']);
            }
            if ($f['status'] === 'blocked') {
                send(['ok' => true, 'statut' => 'envoyee']); // rien n'est révélé à celui qui est bloqué
            }
            if ($f['status'] === 'pending' && (int)$f['target_id'] === (int)$me['id']) {
                // Elle m'avait déjà demandé : demande croisée, amitié acceptée.
                db()->prepare('UPDATE res_friendships SET status = \'accepted\', updated_at = ? WHERE id = ?')->execute([now_ms(), $f['id']]);
                send(['ok' => true, 'statut' => 'acceptee']);
            }
            db()->prepare('UPDATE res_friendships SET requester_id = ?, target_id = ?, status = \'pending\', updated_at = ? WHERE id = ?')
                ->execute([$me['id'], $other['id'], now_ms(), $f['id']]);
            send(['ok' => true, 'statut' => 'envoyee']);
        }
        db()->prepare('INSERT INTO res_friendships (requester_id, target_id, status, created_at, updated_at) VALUES (?, ?, \'pending\', ?, ?)')
            ->execute([$me['id'], $other['id'], now_ms(), now_ms()]);
        send(['ok' => true, 'statut' => 'envoyee']);
    }

    case 'GET amis': {
        $me = require_account();
        $id = (int)$me['id'];
        $st = db()->prepare('SELECT * FROM res_friendships WHERE requester_id = ? OR target_id = ? ORDER BY updated_at DESC');
        $st->execute([$id, $id]);
        $out = [];
        foreach ($st->fetchAll() as $f) {
            $otherId = (int)$f['requester_id'] === $id ? (int)$f['target_id'] : (int)$f['requester_id'];
            $sent = (int)$f['requester_id'] === $id;
            $status = $f['status'];
            // Bloqué par l'autre : vu comme une demande toujours en attente ; refusée : seul le demandeur ne la voit plus.
            if ($status === 'blocked' && (int)$f['blocked_by'] !== $id) {
                $status = 'pending';
            }
            if ($status === 'refused' && $sent) {
                $status = 'pending';
            }
            if ($status === 'refused') {
                continue;
            }
            $o = db()->prepare('SELECT * FROM res_accounts WHERE id = ?');
            $o->execute([$otherId]);
            $other = $o->fetch();
            if (!$other) {
                continue;
            }
            $row = ['id' => (int)$f['id'], 'statut' => $status, 'envoyee' => $sent, 'ami' => public_account($other), 'majLe' => (int)$f['updated_at']];
            if ($status === 'accepted') {
                $sh = db()->prepare('SELECT type, ref, label, rights FROM res_shares WHERE owner_id = ? AND friend_id = ?');
                $sh->execute([$otherId, $id]);
                $row['partagesRecus'] = $sh->fetchAll();
                $sh->execute([$id, $otherId]);
                $row['partagesDonnes'] = $sh->fetchAll();
                if (shared($otherId, $id, 'presence')) {
                    $p = db()->prepare('SELECT state, avatar, updated_at FROM res_presence WHERE account_id = ?');
                    $p->execute([$otherId]);
                    $pr = $p->fetch();
                    // Plus de signe de vie depuis 3 minutes : hors ligne.
                    $row['presence'] = $pr && (int)$pr['updated_at'] > now_ms() - 180000
                        ? ['etat' => $pr['state'], 'avatar' => $pr['avatar'], 'majLe' => (int)$pr['updated_at']]
                        : ['etat' => 'hors_ligne'];
                }
            }
            $out[] = $row;
        }
        send(['ok' => true, 'amis' => $out]);
    }

    case 'POST ami-repondre': {
        $me = require_account();
        $a = body();
        $st = db()->prepare('SELECT * FROM res_friendships WHERE id = ? AND (requester_id = ? OR target_id = ?)');
        $st->execute([int_arg($a, 'id'), $me['id'], $me['id']]);
        $f = $st->fetch();
        if (!$f) {
            fail(404, 'Demande introuvable.');
        }
        $answer = str_arg($a, 'reponse', 12);
        if ($answer === 'bloquer') {
            db()->prepare('UPDATE res_friendships SET status = \'blocked\', blocked_by = ?, updated_at = ? WHERE id = ?')->execute([$me['id'], now_ms(), $f['id']]);
            $other = (int)$f['requester_id'] === (int)$me['id'] ? (int)$f['target_id'] : (int)$f['requester_id'];
            db()->prepare('DELETE FROM res_shares WHERE (owner_id = ? AND friend_id = ?) OR (owner_id = ? AND friend_id = ?)')->execute([$me['id'], $other, $other, $me['id']]);
            send(['ok' => true]);
        }
        if ($answer === 'debloquer' && $f['status'] === 'blocked' && (int)$f['blocked_by'] === (int)$me['id']) {
            db()->prepare('DELETE FROM res_friendships WHERE id = ?')->execute([$f['id']]);
            send(['ok' => true]);
        }
        if ($f['status'] !== 'pending' || (int)$f['target_id'] !== (int)$me['id']) {
            fail(400, 'Seule la personne qui reçoit la demande peut y répondre.');
        }
        if (!in_array($answer, ['accepter', 'refuser'], true)) {
            fail(400, 'Réponse inconnue.');
        }
        db()->prepare('UPDATE res_friendships SET status = ?, updated_at = ? WHERE id = ?')->execute([$answer === 'accepter' ? 'accepted' : 'refused', now_ms(), $f['id']]);
        send(['ok' => true]);
    }

    case 'POST ami-retirer': {
        $me = require_account();
        $a = body();
        $st = db()->prepare('SELECT * FROM res_friendships WHERE id = ? AND (requester_id = ? OR target_id = ?)');
        $st->execute([int_arg($a, 'id'), $me['id'], $me['id']]);
        $f = $st->fetch();
        if (!$f) {
            fail(404, 'Ami introuvable.');
        }
        $other = (int)$f['requester_id'] === (int)$me['id'] ? (int)$f['target_id'] : (int)$f['requester_id'];
        db()->prepare('DELETE FROM res_shares WHERE (owner_id = ? AND friend_id = ?) OR (owner_id = ? AND friend_id = ?)')->execute([$me['id'], $other, $other, $me['id']]);
        db()->prepare('DELETE FROM res_items WHERE (sender_id = ? AND recipient_id = ?) OR (sender_id = ? AND recipient_id = ?)')->execute([$me['id'], $other, $other, $me['id']]);
        db()->prepare('DELETE FROM res_friendships WHERE id = ?')->execute([$f['id']]);
        send(['ok' => true]);
    }

    // --- Partages et présence ---------------------------------------------------------------------------------------

    case 'POST partage': {
        $me = require_account();
        $a = body();
        $friend = int_arg($a, 'ami');
        $type = str_arg($a, 'type', 12);
        $ref = str_arg($a, 'ref', 120);
        if (!are_friends((int)$me['id'], $friend)) {
            fail(403, 'Vous n\'êtes pas amis.');
        }
        if (!in_array($type, ['presence', 'liste', 'dossier'], true)) {
            fail(400, 'Type de partage inconnu.');
        }
        if (empty($a['actif'])) {
            db()->prepare('DELETE FROM res_shares WHERE owner_id = ? AND friend_id = ? AND type = ? AND ref = ?')->execute([$me['id'], $friend, $type, $ref]);
            send(['ok' => true]);
        }
        $rights = str_arg($a, 'droits', 10) === 'ecriture' && $type === 'liste' ? 'ecriture' : 'lecture';
        db()->prepare('REPLACE INTO res_shares (owner_id, friend_id, type, ref, label, rights, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)')
            ->execute([$me['id'], $friend, $type, $ref, str_arg($a, 'label', 160), $rights, now_ms()]);
        send(['ok' => true]);
    }

    case 'POST presence': {
        $me = require_account();
        $a = body();
        $state = str_arg($a, 'etat', 20);
        if (!in_array($state, ['en_ligne', 'absent', 'ne_pas_deranger'], true)) {
            fail(400, 'État inconnu.');
        }
        db()->prepare('REPLACE INTO res_presence (account_id, state, avatar, updated_at) VALUES (?, ?, ?, ?)')
            ->execute([$me['id'], $state, str_arg($a, 'avatar', 200), now_ms()]);
        send(['ok' => true]);
    }

    // --- Boîte aux lettres chiffrée ---------------------------------------------------------------------------------

    case 'POST envoyer': {
        $me = require_account();
        $a = body();
        $to = int_arg($a, 'a');
        $kind = str_arg($a, 'type', 20);
        $payload = (string)($a['contenu'] ?? '');
        $nonce = str_arg($a, 'nonce', 40);
        if (!preg_match('/^[a-z][a-z-]{0,19}$/', $kind) || $payload === '' || strlen($payload) > MAX_ITEM || base64_decode($payload, true) === false) {
            fail(400, 'Envoi invalide.');
        }
        if (!are_friends((int)$me['id'], $to)) {
            fail(403, 'Vous n\'êtes pas amis.');
        }
        rate('envoi:' . $me['id'], 900, 3600 * 1000, 'Trop d\'envois cette heure-ci.');
        $q = db()->prepare('SELECT COALESCE(SUM(bytes), 0) FROM res_items WHERE recipient_id = ?');
        $q->execute([$to]);
        if ((int)$q->fetchColumn() + strlen($payload) > MAX_PENDING_BYTES) {
            fail(507, 'La boîte de ton ami est pleine : il doit d\'abord relever ses envois.');
        }
        db()->prepare('INSERT INTO res_items (sender_id, recipient_id, kind, ref, nonce, payload, bytes, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
            ->execute([$me['id'], $to, $kind, str_arg($a, 'ref', 120), $nonce, $payload, strlen($payload), now_ms()]);
        send(['ok' => true, 'id' => (int)db()->lastInsertId()]);
    }

    case 'GET recevoir': {
        $me = require_account();
        $since = (int)($_GET['depuis'] ?? 0);
        $st = db()->prepare('SELECT id, sender_id, kind, ref, nonce, payload, created_at FROM res_items WHERE recipient_id = ? AND id > ? ORDER BY id LIMIT 100');
        $st->execute([$me['id'], $since]);
        $items = array_map(fn($r) => [
            'id' => (int)$r['id'], 'de' => (int)$r['sender_id'], 'type' => $r['kind'], 'ref' => $r['ref'],
            'nonce' => $r['nonce'], 'contenu' => $r['payload'], 'le' => (int)$r['created_at'],
        ], $st->fetchAll());
        $f = db()->prepare('SELECT COALESCE(MAX(updated_at), 0) FROM res_friendships WHERE requester_id = ? OR target_id = ?');
        $f->execute([$me['id'], $me['id']]);
        send(['ok' => true, 'envois' => $items, 'amisMajLe' => (int)$f->fetchColumn()]);
    }

    // Envois relevés et enregistrés par Jarvis : effacés du serveur.
    case 'POST accuse': {
        $me = require_account();
        $a = body();
        db()->prepare('DELETE FROM res_items WHERE recipient_id = ? AND id <= ?')->execute([$me['id'], int_arg($a, 'jusqua')]);
        send(['ok' => true]);
    }

    default:
        fail(404, 'Route inconnue.');
}
