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

export function pickPokemon(taken, rand = Math.random) {
  const free = POKEMON.map((_, i) => i + 1).filter((n) => !taken.has(n));
  const from = free.length ? free : POKEMON.map((_, i) => i + 1);
  return from[Math.min(from.length - 1, Math.floor(rand() * from.length))];
}

export const expressionFor = (status) => EXPRESSION[status] ?? "Normal";

export const portraitFile = (n, expression) => `${String(n).padStart(4, "0")}/${expression}.png`;

export const portraitUrl = (n, expression) => `${SOURCE}/${portraitFile(n, expression)}`;

// Some Pokemon lack some expressions upstream; Normal exists for all 151.
export const portraitUrls = (n, expression) => (expression === "Normal" ? [portraitUrl(n, "Normal")] : [portraitUrl(n, expression), portraitUrl(n, "Normal")]);

export const EXPRESSIONS = ["Worried", "Determined", "Joyous", "Normal"];
