-- Listes créées par les invités (cadeaux, idées, courses…) : sources dont l'invité est propriétaire.
-- Lien avec la personne (famille, ami…) : rangement des listes dans la mémoire de Jarvis.

ALTER TABLE users ADD COLUMN relation VARCHAR(20) NOT NULL DEFAULT 'ami';
ALTER TABLE sources ADD COLUMN owner_user INT NULL;
ALTER TABLE sources ADD KEY idx_sources_owner (owner_user);
