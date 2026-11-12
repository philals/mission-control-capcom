// SG-1 personas for agents. Pure data: scripts/gen-avatars.mjs reads this file too.
// Drop a real photo at <mod>/avatars/<id>.jpg or .png to replace a character's drawn portrait.

export const CHARACTERS = [
  { id: "oneill", name: "O'Neill", color: "#6fa8dc", bg: "#33475a", skin: "#e2bd9a", hair: "#b9bec4", style: "cap", cap: "#1d2328", uni: "#5b6b4a" },
  { id: "carter", name: "Carter", color: "#e0c24a", bg: "#2f4a57", skin: "#f0cdb0", hair: "#e6c35c", style: "bob", uni: "#5b6b4a" },
  { id: "daniel", name: "Daniel", color: "#9ad07a", bg: "#3d4a3a", skin: "#ecc7a6", hair: "#6b4a33", style: "short", glasses: true, uni: "#6a6e50" },
  { id: "tealc", name: "Teal'c", color: "#e0a63a", bg: "#2d3340", skin: "#8b5a3c", hair: null, style: "bald", mark: true, uni: "#2a2f38" },
  { id: "hammond", name: "Hammond", color: "#b48ee0", bg: "#2f3350", skin: "#e0b896", hair: "#aab0b8", style: "fringe", uni: "#27406b", stars: true },
];

// Seven chevrons round the portrait; the ring file for a task with `total` steps (6 or 7) and the steps `mask` done.
export const ringFile = (total, mask) => `rings/r${total}-${mask}.svg`;
export const dimFile = (total) => `rings/dim${total}.svg`;
