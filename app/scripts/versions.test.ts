import assert from "node:assert/strict";
import { test } from "node:test";
import { bodyBullets, releaseNotes } from "./lib/notes.ts";
import { compareVersions, devSeries, nextDevNumber, sortTags, tagVersion, versionLabel } from "../src/versions.ts";

test("tags map to semver", () => {
  assert.equal(tagVersion("v0.2.0"), "0.2.0");
  assert.equal(tagVersion("v0.2.0.3"), "0.2.0-dev.3");
  assert.equal(tagVersion("v0.2"), null);
  assert.equal(versionLabel("0.2.0-dev.3"), "0.2.0.3");
  assert.equal(versionLabel("0.2.0-preview.4"), "0.2.0.4-preview");
});

test("dev builds sit between releases", () => {
  assert.ok(compareVersions("0.2.0-dev.3", "0.1.0") > 0);
  assert.ok(compareVersions("0.2.0-dev.3", "0.2.0-dev.10") < 0);
  assert.ok(compareVersions("0.2.0", "0.2.0-dev.10") > 0);
  assert.deepEqual(sortTags(["v0.2.0", "v0.2.0.2", "v0.1.0", "junk", "v0.2.0.10"]), ["v0.1.0", "v0.2.0.2", "v0.2.0.10", "v0.2.0"]);
  assert.equal(devSeries("0.1.0"), "0.2.0");
  assert.equal(nextDevNumber("0.2.0", ["v0.2.0.1", "v0.2.0.7", "v0.3.0.9"]), 8);
});

test("release notes nest commit body bullets", () => {
  assert.deepEqual(bodyBullets("Intro\n\n- one\n  continued\n- two\n\nCo-Authored-By: x"), ["one continued", "two"]);
  const md = releaseNotes({ tag: "v0.2.0.1", previous: "v0.1.0", commits: [{ hash: "abc", subject: "Add X", body: "- detail" }], release: "0.1.0" });
  assert.match(md, /^Dev build of 0.2.0/);
  assert.match(md, /## Changes since 0.1.0\n\n- Add X \(abc\)\n  - detail/);
});
