// Compétence d'exemple (voir README.md) : pile ou face, et un dé à N faces.
module.exports = {
  name: 'Pile ou face et dé',
  examples: ['pile ou face', 'lance un dé', 'lance un dé à 20 faces'],
  match: /^(?:pile ou face|lance une piece|(?:lance|jette) un de(?: a (\d+) faces)?)$/,
  async run({ match }) {
    if (/^(?:pile|lance une piece)/.test(match[0])) return Math.random() < 0.5 ? 'Pile !' : 'Face !';
    const faces = Math.max(2, Math.min(1000, Number(match[1]) || 6));
    const n = 1 + Math.floor(Math.random() * faces);
    return { say: `${n} !`, display: `Dé à ${faces} faces : ${n}` };
  }
};
