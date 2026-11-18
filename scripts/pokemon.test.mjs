import test from "node:test";
import assert from "node:assert/strict";
import { POKEMON, nameOf, validNumber, pickPokemon, expressionFor, portraitFile, portraitUrl, portraitUrls, EXPRESSIONS, assignable, typeColor } from "../herdr-mod/capcom-agents/pokemon.js";

test("the first 151 in order", () => {
  assert.equal(POKEMON.length, 151);
  assert.equal(POKEMON[0], "Bulbasaur");
  assert.equal(POKEMON[3], "Charmander");
  assert.equal(POKEMON[24], "Pikachu");
  assert.equal(POKEMON[150], "Mew");
  assert.equal(nameOf(25), "Pikachu");
});

test("validNumber accepts integers 1 to 151 only", () => {
  for (const ok of [1, 151]) assert.equal(validNumber(ok), true);
  for (const bad of [0, 152, 1.5, "4", null, undefined, NaN]) assert.equal(validNumber(bad), false);
});

test("expression follows status, unknown is Normal", () => {
  assert.equal(expressionFor("blocked"), "Worried");
  assert.equal(expressionFor("working"), "Determined");
  assert.equal(expressionFor("done"), "Joyous");
  assert.equal(expressionFor("idle"), "Normal");
  assert.equal(expressionFor("error"), "Normal");
  assert.equal(expressionFor(undefined), "Normal");
});

test("portrait file and url", () => {
  assert.equal(portraitFile(4, "Worried"), "0004/Worried.png");
  assert.equal(portraitUrl(25, "Joyous"), "https://raw.githubusercontent.com/PMDCollab/SpriteCollab/master/portrait/0025/Joyous.png");
});

test("pickPokemon starts at 1 and goes on to the next number", () => {
  assert.equal(pickPokemon(new Set()), 1);
  assert.equal(pickPokemon(new Set(), 1), 2);
  assert.equal(pickPokemon(new Set(), 24), 25);
});

test("pickPokemon wraps from 151 to 1", () => {
  assert.equal(pickPokemon(new Set(), 151), 1);
});

test("pickPokemon skips numbers a live agent has", () => {
  assert.equal(pickPokemon(new Set([2, 3]), 1), 4);
  assert.equal(pickPokemon(new Set([1, 2, 3]), 150), 151);
  assert.equal(pickPokemon(new Set([151, 1]), 150), 2);
});

test("pickPokemon still answers when all 151 are taken", () => {
  const all = new Set(Array.from({ length: 151 }, (_, i) => i + 1));
  assert.equal(pickPokemon(all, 10), 11);
  assert.equal(pickPokemon(all, 151), 1);
});

test("every one of the 151 can be assigned", () => {
  assert.equal(assignable(88), true);
  assert.equal(assignable(151), true);
  assert.equal(assignable(152), false);
});

test("portraitUrls tries similar expressions, then Normal", () => {
  const u = (e) => portraitUrl(17, e);
  assert.deepEqual(portraitUrls(17, "Worried"), ["Worried", "Surprised", "Pain", "Stunned", "Sad", "Normal"].map(u));
  assert.deepEqual(portraitUrls(17, "Determined"), ["Determined", "Inspired", "Normal"].map(u));
  assert.deepEqual(portraitUrls(17, "Joyous"), ["Joyous", "Happy", "Inspired", "Normal"].map(u));
  assert.deepEqual(portraitUrls(17, "Normal"), [u("Normal")]);
});

test("every status expression is one that gets prefetched", () => {
  for (const status of ["blocked", "working", "done", "idle"]) assert.ok(EXPRESSIONS.includes(expressionFor(status)));
});

test("typeColor gives each of the 151 its first type's colour", () => {
  assert.equal(typeColor(4), "#ee8130");
  assert.equal(typeColor(25), "#f7d02c");
  assert.equal(typeColor(92), "#735797");
  assert.equal(typeColor(149), "#6f35fc");
  for (let n = 1; n <= 151; n++) assert.match(typeColor(n), /^#[0-9a-f]{6}$/);
  assert.equal(typeColor(152), null);
});
