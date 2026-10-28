// Compétence vocale locale sans IA : contrôle de la surveillance continue de Claude Code et AGY.

module.exports = {
  name: 'Surveillance Claude & AGY',
  examples: [
    'active la surveillance de Claude',
    'surveille Claude et AGY',
    'désactive la surveillance IA',
    'état de la surveillance IA'
  ],
  match: /^(?:active(?: la)? surveillance (?:de claude|d agy|des ia|ia|claude et agy)|surveille (?:claude|agy|claude et agy)|desactive(?: la)? surveillance (?:de claude|d agy|des ia|ia|claude et agy)|(?:etat|statut)(?: de la)? surveillance (?:de claude|d agy|des ia|ia|claude et agy))$/,
  async run({ text, orch }) {
    if (!orch || !orch.config) return 'Configuration indisponible.';

    if (/^desactive/.test(text)) {
      orch.config.update({ aiWatch: { enabled: false } });
      return 'La surveillance permanente de Claude Code et AGY est désactivée.';
    }

    if (/^(?:etat|statut)/.test(text)) {
      const cfg = (orch.config.get && orch.config.get('aiWatch')) || {};
      return cfg.enabled !== false
        ? "La surveillance permanente de Claude Code et AGY est active. Je t'alerte dès qu'un prompt se termine ou attend ta réponse."
        : 'La surveillance permanente de Claude Code et AGY est actuellement désactivée.';
    }

    // Par défaut : activer la surveillance
    orch.config.update({ aiWatch: { enabled: true } });
    return "La surveillance permanente de Claude Code et AGY est activée. Je te préviendrai dès qu'un prompt est terminé ou qu'une réponse est attendue.";
  }
};
