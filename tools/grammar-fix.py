# Correction orthographique et grammaticale hors ligne (chantier v5, étape 7) avec Grammalecte.
# Entrée : texte UTF-8 sur stdin. Sortie : JSON { "text": corrigé, "fixes": [...] } sur stdout.
# La première suggestion de chaque erreur est appliquée, de la fin vers le début du paragraphe
# pour que les positions restent justes. Aucun réseau, aucune IA.

import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "grammalecte"))
sys.stdout.reconfigure(encoding="utf-8")
sys.stdin.reconfigure(encoding="utf-8")

import grammalecte  # noqa: E402

checker = grammalecte.GrammarChecker("fr")


def errors_of(paragraph):
    gram, spell = checker.getParagraphErrors(paragraph, bSpellSugg=True)
    errors = []
    for e in gram:
        if e.get("aSuggestions"):
            errors.append((e["nStart"], e["nEnd"], e["aSuggestions"][0], e.get("sMessage", "")))
    for e in spell:
        if e.get("aSuggestions"):
            errors.append((e["nStart"], e["nEnd"], e["aSuggestions"][0], "orthographe"))
    errors.sort(key=lambda x: (x[0], x[1]))
    return errors


# Correction itérative : la faute la plus à gauche est corrigée, puis le paragraphe est
# réanalysé. Deux règles liées (« il sont » : accord du sujet ou du verbe) ne s'appliquent
# ainsi jamais ensemble (ce qui donnerait « ils est »).
def fix_paragraph(paragraph):
    if not paragraph.strip():
        return paragraph, []
    fixes = []
    done = set()
    for _ in range(40):
        pending = [e for e in errors_of(paragraph) if (e[0], paragraph[e[0]:e[1]], e[2]) not in done]
        if not pending:
            break
        start, end, sugg, msg = pending[0]
        before = paragraph[start:end]
        done.add((start, before, sugg))
        if before == sugg:
            continue
        paragraph = paragraph[:start] + sugg + paragraph[end:]
        fixes.append({"avant": before, "apres": sugg, "message": msg})
    return paragraph, fixes


def main():
    text = sys.stdin.read()
    out = []
    all_fixes = []
    for paragraph in text.split("\n"):
        fixed, fixes = fix_paragraph(paragraph)
        out.append(fixed)
        all_fixes.extend(fixes)
    print(json.dumps({"text": "\n".join(out), "fixes": all_fixes}, ensure_ascii=False))


if __name__ == "__main__":
    main()
