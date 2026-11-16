import test from "node:test";
import assert from "node:assert/strict";
import { POKEMON, nameOf, validNumber, pickPokemon, expressionFor, portraitFile, portraitUrl, portraitUrls } from "../herdr-mod/capcom-agents/pokemon.js";

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

test("pickPokemon takes the first free number when rand is 0", () => {
  assert.equal(pickPokemon(new Set([1, 2, 3]), () => 0), 4);
});

test("pickPokemon never returns a taken number", () => {
  const taken = new Set(Array.from({ length: 150 }, (_, i) => i + 1));
  for (let i = 0; i < 200; i++) assert.equal(pickPokemon(taken), 151);
});

test("pickPokemon reuses one when all 151 are taken", () => {
  const n = pickPokemon(new Set(Array.from({ length: 151 }, (_, i) => i + 1)));
  assert.ok(Number.isInteger(n) && n >= 1 && n <= 151);
});

test("portraitUrls falls back to Normal because some Pokemon lack an expression", () => {
  assert.deepEqual(portraitUrls(17, "Joyous"), [portraitUrl(17, "Joyous"), portraitUrl(17, "Normal")]);
  assert.deepEqual(portraitUrls(17, "Normal"), [portraitUrl(17, "Normal")]);
});
