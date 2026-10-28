<?php
// Jarvis Réseau — mises à jour du schéma (appelé par deploy.ps1, jeton X-Deploy-Token). Table de suivi propre
// (res_schema_migrations) : celle du site Nomade, dans la même base, n'est jamais touchée.

declare(strict_types=1);
require __DIR__ . '/lib.php';

if ($_SERVER['REQUEST_METHOD'] !== 'POST' || !hash_equals((string)cfg()['deploy_token'], (string)($_SERVER['HTTP_X_DEPLOY_TOKEN'] ?? ''))) {
    fail(403, 'Refusé.');
}

$pdo = db();
$pdo->exec('CREATE TABLE IF NOT EXISTS res_schema_migrations (name VARCHAR(120) PRIMARY KEY, applied_at BIGINT NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4');
$done = $pdo->query('SELECT name FROM res_schema_migrations')->fetchAll(PDO::FETCH_COLUMN);
$files = glob(__DIR__ . '/../migrations/*.sql') ?: [];
sort($files);
$applied = [];
foreach ($files as $file) {
    $name = basename($file);
    if (in_array($name, $done, true)) {
        continue;
    }
    $sql = preg_replace('/^\s*--.*$/m', '', (string)file_get_contents($file));
    foreach (preg_split('/;\s*$/m', $sql) as $stmt) {
        if (trim($stmt) !== '') {
            $pdo->exec($stmt);
        }
    }
    $pdo->prepare('INSERT INTO res_schema_migrations (name, applied_at) VALUES (?, ?)')->execute([$name, now_ms()]);
    $applied[] = $name;
}
send(['ok' => true, 'applied' => $applied, 'total' => count($files)]);
