# Jarvis Nomade — site distant

Application web (téléphone et PC) qui montre les données que Jarvis partage (Administration › Nomade) et reçoit
ce qu'on y ajoute. Hébergement : Hostinger mutualisé (PHP 7.4 ou plus, MySQL/MariaDB). Jarvis l'appelle en HTTPS
(`orch/nomade`), aucun port n'est ouvert sur le PC.

| Chemin | Rôle |
|---|---|
| `index.html`, `app.js`, `app.css`, `sw.js`, `manifest.json` | l'application (connexion mémorisée 90 jours, installable, lecture hors ligne) |
| `api/index.php` | API : `?r=login`, `me`, `records`, `edit`, `inbox`… (site, cookie) ; `pair`, `push`, `pull`, `drop`, `account`… (Jarvis, clé d'appareil) |
| `api/migrate.php` | mises à jour du schéma, jeton de déploiement exigé |
| `migrations/*.sql` | schéma (une migration = un fichier numéroté, jamais modifié une fois déployé) |
| `private/config.php` | accès à la base et jetons — **généré par `deploy.ps1`, jamais versionné** |

## Déployer

1. Remplir `%USERPROFILE%\.jarvis-nomade\deploy.env` (FTP, base MySQL, adresse du site). Le FTP est toujours chiffré ;
   si le certificat du serveur ne porte pas le nom de `FTP_HOST`, `FTP_TLS_NAME` donne un nom qu'il couvre.
2. `pwsh web-nomade/deploy.ps1` : envoie les fichiers par FTP, génère `private/config.php`, applique les migrations,
   et laisse à Jarvis le code d'installation (`data/nomade/install-token.txt`).
3. Administration › Nomade : adresse, identifiant et mot de passe → « Relier au site » (crée le compte la première fois).

`pwsh web-nomade/deploy.ps1 -Only app.js,app.css` renvoie seulement quelques fichiers.
