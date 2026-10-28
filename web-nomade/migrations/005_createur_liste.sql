-- Créateur d'une liste à fiches demandée par un invité (la liste appartient à Jarvis, le créateur décide de qui d'autre la voit).
-- Pour les listes simples d'un invité, le créateur reste owner_user.

ALTER TABLE sources ADD COLUMN creator_user INT NULL;
