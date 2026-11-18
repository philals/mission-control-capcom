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

export const assignable = validNumber;

// The next one after `last`, wrapping from 151 to 1, skipping any a live agent has; with all taken, just the next.
export function pickPokemon(taken, last = 0) {
  const total = POKEMON.length;
  for (let i = 1; i <= total; i++) {
    const n = ((last + i - 1) % total) + 1;
    if (!taken.has(n)) return n;
  }
  return (last % total) + 1;
}

export const expressionFor = (status) => EXPRESSION[status] ?? "Normal";

export const portraitFile = (n, expression) => `${String(n).padStart(4, "0")}/${expression}.png`;

export const portraitUrl = (n, expression) => `${SOURCE}/${portraitFile(n, expression)}`;

// Some Pokemon lack some expressions upstream. Similar ones are tried in order; Normal exists for all 151.
const SIMILAR = { Worried: ["Surprised", "Pain", "Stunned", "Sad"], Determined: ["Inspired"], Joyous: ["Happy", "Inspired"] };

export const portraitUrls = (n, expression) => [expression, ...(SIMILAR[expression] ?? []), ...(expression === "Normal" ? [] : ["Normal"])].map((e) => portraitUrl(n, e));

export const EXPRESSIONS = ["Worried", "Determined", "Joyous", "Normal"];

// Shown in place of a portrait until it downloads.
export const BALL_FILE = "pokeball.png";
export const BALL_URL = "https://raw.githubusercontent.com/PokeAPI/sprites/master/sprites/items/poke-ball.png";

// Each one's first type, one letter per Pokemon in order, and that type's colour.
const TYPE_OF = "GGGFFFWWWBBBBBBNNNNNNNPPEEDDPPPPPPYYFFNNPPGGGBBBBDDNNWWXXFFWWWSSSXXXGGGWWRRRFFWWEENNNWWPPWWHHHRSSWWEEGGDDXXNPPDDNGNWWWWWWSBIEFBNWWWNNWEFNRRRRRNIEFZZZSS";
const TYPE_COLORS = {
  N: "#a8a77a", F: "#ee8130", W: "#6390f0", E: "#f7d02c", G: "#7ac74c", I: "#96d9d6", X: "#c22e28", P: "#a33ea1",
  D: "#e2bf65", S: "#f95587", B: "#a6b91a", R: "#b6a136", H: "#735797", Z: "#6f35fc", Y: "#d685ad",
};

export const typeColor = (n) => (validNumber(n) ? TYPE_COLORS[TYPE_OF[n - 1]] : null);
