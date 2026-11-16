// The first 151 Pokemon for the Agents panel's Pokemon mode. Pure data and helpers: scripts/pokemon.test.mjs imports this.

export const POKEMON = [
  "Bulbasaur", "Ivysaur", "Venusaur", "Charmander", "Charmeleon", "Charizard", "Squirtle", "Wartortle", "Blastoise",
  "Caterpie", "Metapod", "Butterfree", "Weedle", "Kakuna", "Beedrill", "Pidgey", "Pidgeotto", "Pidgeot", "Rattata",
  "Raticate", "Spearow", "Fearow", "Ekans", "Arbok", "Pikachu", "Raichu", "Sandshrew", "Sandslash", "Nidoran F",
  "Nidorina", "Nidoqueen", "Nidoran M", "Nidorino", "Nidoking", "Clefairy", "Clefable", "Vulpix", "Ninetales",
  "Jigglypuff", "Wigglytuff", "Zubat", "Golbat", "Oddish", "Gloom", "Vileplume", "Paras", "Parasect", "Venonat",
  "Venomoth", "Diglett", "Dugtrio", "Meowth", "Persian", "Psyduck", "Golduck", "Mankey", "Primeape", "Growlithe",
  "Arcanine", "Poliwag", "Poliwhirl", "Poliwrath", "Abra", "Kadabra", "Alakazam", "Machop", "Machoke", "Machamp",
  "Bellsprout", "Weepinbell", "Victreebel", "Tentacool", "Tentacruel", "Geodude", "Graveler", "Golem", "Ponyta",
  "Rapidash", "Slowpoke", "Slowbro", "Magnemite", "Magneton", "Farfetch'd", "Doduo", "Dodrio", "Seel", "Dewgong",
  "Grimer", "Muk", "Shellder", "Cloyster", "Gastly", "Haunter", "Gengar", "Onix", "Drowzee", "Hypno", "Krabby",
  "Kingler", "Voltorb", "Electrode", "Exeggcute", "Exeggutor", "Cubone", "Marowak", "Hitmonlee", "Hitmonchan",
  "Lickitung", "Koffing", "Weezing", "Rhyhorn", "Rhydon", "Chansey", "Tangela", "Kangaskhan", "Horsea", "Seadra",
  "Goldeen", "Seaking", "Staryu", "Starmie", "Mr. Mime", "Scyther", "Jynx", "Electabuzz", "Magmar", "Pinsir",
  "Tauros", "Magikarp", "Gyarados", "Lapras", "Ditto", "Eevee", "Vaporeon", "Jolteon", "Flareon", "Porygon",
  "Omanyte", "Omastar", "Kabuto", "Kabutops", "Aerodactyl", "Snorlax", "Articuno", "Zapdos", "Moltres", "Dratini",
  "Dragonair", "Dragonite", "Mewtwo", "Mew",
];

const EXPRESSION = { blocked: "Worried", working: "Determined", done: "Joyous" };
const SOURCE = "https://raw.githubusercontent.com/PMDCollab/SpriteCollab/master/portrait";

export const nameOf = (n) => POKEMON[n - 1];

export const validNumber = (v) => Number.isInteger(v) && v >= 1 && v <= POKEMON.length;

// The 33 with no Worried, Determined or Joyous portrait upstream (checked against PMDCollab SpriteCollab): they would
// look the same working, waiting and done, so they are never assigned.
export const STATIC = new Set([20, 33, 45, 47, 56, 57, 68, 70, 71, 73, 74, 76, 82, 88, 89, 91, 94, 95, 102, 107, 108, 110, 117, 119, 120, 121, 122, 124, 127, 139, 144, 145, 146]);

export const assignable = (n) => validNumber(n) && !STATIC.has(n);

export function pickPokemon(taken, rand = Math.random) {
  const all = POKEMON.map((_, i) => i + 1).filter(assignable);
  const free = all.filter((n) => !taken.has(n));
  const from = free.length ? free : all;
  return from[Math.min(from.length - 1, Math.floor(rand() * from.length))];
}

export const expressionFor = (status) => EXPRESSION[status] ?? "Normal";

export const portraitFile = (n, expression) => `${String(n).padStart(4, "0")}/${expression}.png`;

export const portraitUrl = (n, expression) => `${SOURCE}/${portraitFile(n, expression)}`;

// Some Pokemon lack some expressions upstream. Similar ones are tried in order; Normal exists for all 151.
const SIMILAR = { Worried: ["Surprised", "Pain", "Stunned", "Sad"], Determined: ["Inspired"], Joyous: ["Happy", "Inspired"] };

export const portraitUrls = (n, expression) => [expression, ...(SIMILAR[expression] ?? []), ...(expression === "Normal" ? [] : ["Normal"])].map((e) => portraitUrl(n, e));

export const EXPRESSIONS = ["Worried", "Determined", "Joyous", "Normal"];
