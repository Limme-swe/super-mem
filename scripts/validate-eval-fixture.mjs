#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const defaultFixture = fileURLToPath(
  new URL("../fixtures/eval/v1.jsonl", import.meta.url),
);
const fixturePath = resolve(process.argv[2] ?? defaultFixture);

const requiredCases = new Map([
  ["supersession-msrv", ["supersession", "current-view"]],
  ["failed-attempt-vs-procedure", ["failed-attempt", "outcome-aware"]],
  ["repository-isolation-auth", ["repo-isolation", "scope"]],
  ["branch-divergence-api", ["branch-divergence", "git-applicability"]],
  ["stale-router-artifact", ["stale-artifact", "git-applicability"]],
  ["exact-rust-error-e0277", ["exact-error", "diagnostic-recall"]],
]);
const allowedStatuses = new Set([
  "current",
  "superseded",
  "validated-procedure",
  "known-failure",
  "exact",
  "compatible",
  "stale",
  "divergent",
  "unversioned",
  "inapplicable",
]);
const excludedStatuses = new Set(["superseded", "divergent", "inapplicable"]);
const oidPattern = /^[0-9a-f]{40}$/;
const stopWords = new Set([
  "and",
  "are",
  "for",
  "from",
  "how",
  "into",
  "not",
  "should",
  "the",
  "this",
  "uses",
  "what",
  "where",
  "which",
  "with",
]);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function uniqueStrings(values, field, caseId) {
  check(Array.isArray(values), `${caseId}: ${field} must be an array`);
  check(
    values.every((value) => typeof value === "string" && value.length > 0),
    `${caseId}: ${field} must contain non-empty strings`,
  );
  check(new Set(values).size === values.length, `${caseId}: ${field} has duplicates`);
}

function tokens(text) {
  return new Set(
    text
      .toLowerCase()
      .match(/[a-z0-9]+/g)
      ?.filter((token) => token.length > 2 && !stopWords.has(token)) ?? [],
  );
}

function lexicalOverlap(query, content) {
  const queryTokens = tokens(query);
  return [...tokens(content)].filter((token) => queryTokens.has(token)).length;
}

function validateGraph(testCase) {
  const { case_id: caseId, repo_graph: graph } = testCase;
  check(object(graph), `${caseId}: repo_graph must be an object`);
  check(
    typeof graph.repo_id === "string" && graph.repo_id.length > 0,
    `${caseId}: repo_graph.repo_id is required`,
  );
  check(object(graph.parents), `${caseId}: repo_graph.parents must be an object`);

  for (const [child, parents] of Object.entries(graph.parents)) {
    check(oidPattern.test(child), `${caseId}: invalid child commit ${child}`);
    uniqueStrings(parents, `parents.${child}`, caseId);
    check(
      parents.every((parent) => oidPattern.test(parent)),
      `${caseId}: invalid parent commit for ${child}`,
    );
  }

  const visiting = new Set();
  const visited = new Set();
  const visit = (commit) => {
    check(!visiting.has(commit), `${caseId}: repository graph contains a cycle at ${commit}`);
    if (visited.has(commit)) return;
    visiting.add(commit);
    for (const parent of graph.parents[commit] ?? []) visit(parent);
    visiting.delete(commit);
    visited.add(commit);
  };
  for (const commit of Object.keys(graph.parents)) visit(commit);
}

function isAncestor(graph, ancestor, descendant) {
  const pending = [descendant];
  const visited = new Set();
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === ancestor) return true;
    if (visited.has(current)) continue;
    visited.add(current);
    pending.push(...(graph.parents[current] ?? []));
  }
  return false;
}

function changedArtifact(recordState, queryState) {
  if (!object(recordState) || !object(queryState)) return false;
  const queryArtifacts = new Map(
    (queryState.artifacts ?? [])
      .filter((artifact) => object(artifact))
      .map((artifact) => [artifact.path, artifact.digest]),
  );
  const artifactChanged = (recordState.artifacts ?? [])
    .filter((artifact) => object(artifact))
    .some(
      (artifact) =>
        queryArtifacts.has(artifact.path) &&
        queryArtifacts.get(artifact.path) !== artifact.digest,
    );
  const querySymbols = new Map(
    (queryState.symbols ?? []).map((symbol) => [
      symbol.qualified_name,
      symbol.signature_digest,
    ]),
  );
  const symbolChanged = (recordState.symbols ?? []).some(
    (symbol) =>
      querySymbols.has(symbol.qualified_name) &&
      querySymbols.get(symbol.qualified_name) !== symbol.signature_digest,
  );
  return artifactChanged || symbolChanged;
}

function deriveStatus(testCase, record, supersededIds) {
  if (record.scope.repo_id !== testCase.query.scope.repo_id) return "inapplicable";
  if (supersededIds.has(record.id)) return "superseded";
  if (record.kind === "attempt" && record.outcome === "failure") return "known-failure";
  if (record.kind === "procedure" && record.outcome === "success") {
    return "validated-procedure";
  }
  if (!object(record.repo_state)) return "current";
  if (changedArtifact(record.repo_state, testCase.query.repo_state)) return "stale";

  const recordCommit = record.repo_state.commit;
  const queryCommit = testCase.query.repo_state?.head;
  if (!recordCommit || !queryCommit) return "unversioned";
  if (recordCommit === queryCommit) return "exact";
  if (isAncestor(testCase.repo_graph, recordCommit, queryCommit)) return "compatible";
  return "divergent";
}

function validateRecord(record, recordIds, testCase) {
  const caseId = testCase.case_id;
  check(object(record), `${caseId}: every record must be an object`);
  check(typeof record.id === "string" && record.id.length > 0, `${caseId}: record id is required`);
  check(!recordIds.has(record.id), `${caseId}: duplicate record id ${record.id}`);
  recordIds.add(record.id);
  check(typeof record.kind === "string" && record.kind.length > 0, `${caseId}/${record.id}: kind is required`);
  check(
    typeof record.content === "string" && record.content.trim().length > 0,
    `${caseId}/${record.id}: content is required`,
  );
  check(object(record.scope), `${caseId}/${record.id}: scope must be an object`);
  check(
    typeof record.scope.repo_id === "string" && record.scope.repo_id.length > 0,
    `${caseId}/${record.id}: scope.repo_id is required`,
  );
  check(
    typeof record.observed_at === "string" && Number.isFinite(Date.parse(record.observed_at)),
    `${caseId}/${record.id}: observed_at must be an ISO timestamp`,
  );
  check(object(record.provenance), `${caseId}/${record.id}: provenance is required`);
  check(
    typeof record.provenance.type === "string" &&
      typeof record.provenance.ref === "string" &&
      record.provenance.ref.length > 0,
    `${caseId}/${record.id}: provenance type and ref are required`,
  );
  if (record.repo_state?.commit !== undefined) {
    check(
      oidPattern.test(record.repo_state.commit),
      `${caseId}/${record.id}: invalid record commit`,
    );
  }
}

function rankingAdvantages(testCase, preferred, lowerRanked) {
  const advantages = [];
  const signature = testCase.query.diagnostic_signature;
  if (
    signature &&
    preferred.diagnostic_signature === signature &&
    lowerRanked.diagnostic_signature !== signature
  ) {
    advantages.push("exact diagnostic");
  }
  if (preferred.outcome === "success" && lowerRanked.outcome !== "success") {
    advantages.push("validated outcome");
  }
  if (preferred.kind === "procedure" && lowerRanked.kind === "attempt") {
    advantages.push("procedure over attempt");
  }
  if (
    lexicalOverlap(testCase.query.text, preferred.content) >
    lexicalOverlap(testCase.query.text, lowerRanked.content)
  ) {
    advantages.push("stronger lexical signal");
  }
  return advantages;
}

function validateCase(testCase, caseIds, globalRecordIds) {
  check(object(testCase), "each JSONL line must contain an object");
  const caseId = testCase.case_id;
  check(typeof caseId === "string" && caseId.length > 0, "case_id is required");
  check(!caseIds.has(caseId), `duplicate case_id ${caseId}`);
  caseIds.add(caseId);
  check(testCase.schema_version === "supermem.eval.v1", `${caseId}: unsupported schema version`);
  uniqueStrings(testCase.labels, "labels", caseId);
  check(
    typeof testCase.description === "string" && testCase.description.trim().length > 0,
    `${caseId}: description is required`,
  );
  validateGraph(testCase);

  check(Array.isArray(testCase.records) && testCase.records.length > 0, `${caseId}: records are required`);
  const recordIds = new Set();
  for (const record of testCase.records) {
    validateRecord(record, recordIds, testCase);
    check(!globalRecordIds.has(record.id), `record id ${record.id} is reused across cases`);
    globalRecordIds.add(record.id);
  }
  const byId = new Map(testCase.records.map((record) => [record.id, record]));
  for (const record of testCase.records) {
    if (record.supersedes !== undefined) {
      check(record.supersedes !== record.id, `${caseId}/${record.id}: cannot supersede itself`);
      check(byId.has(record.supersedes), `${caseId}/${record.id}: supersedes unknown record`);
      check(
        Date.parse(record.observed_at) > Date.parse(byId.get(record.supersedes).observed_at),
        `${caseId}/${record.id}: correction must be newer than the superseded record`,
      );
    }
  }

  check(object(testCase.query), `${caseId}: query must be an object`);
  check(
    typeof testCase.query.text === "string" && testCase.query.text.trim().length > 0,
    `${caseId}: query text is required`,
  );
  check(object(testCase.query.scope), `${caseId}: query scope is required`);
  check(
    testCase.query.scope.repo_id === testCase.repo_graph.repo_id,
    `${caseId}: query and repository graph scopes differ`,
  );
  check(
    Number.isInteger(testCase.query.budget_tokens) && testCase.query.budget_tokens > 0,
    `${caseId}: budget_tokens must be a positive integer`,
  );
  if (testCase.query.repo_state?.head !== undefined) {
    check(oidPattern.test(testCase.query.repo_state.head), `${caseId}: invalid query head`);
  }

  const expected = testCase.expected;
  check(object(expected), `${caseId}: expected assertions are required`);
  uniqueStrings(expected.must_return, "expected.must_return", caseId);
  uniqueStrings(expected.must_not_return, "expected.must_not_return", caseId);
  const required = new Set(expected.must_return);
  const forbidden = new Set(expected.must_not_return);
  for (const id of [...required, ...forbidden]) {
    check(byId.has(id), `${caseId}: expected assertion references unknown record ${id}`);
    check(!(required.has(id) && forbidden.has(id)), `${caseId}: ${id} is both required and forbidden`);
  }
  check(object(expected.status), `${caseId}: expected.status must be an object`);
  for (const [id, status] of Object.entries(expected.status)) {
    check(byId.has(id), `${caseId}: status references unknown record ${id}`);
    check(allowedStatuses.has(status), `${caseId}/${id}: unsupported status ${status}`);
  }
  check(Array.isArray(expected.rank_before), `${caseId}: expected.rank_before must be an array`);
  check(
    typeof expected.reason === "string" && expected.reason.trim().length > 0,
    `${caseId}: expected.reason is required`,
  );

  const supersededIds = new Set(
    testCase.records.map((record) => record.supersedes).filter(Boolean),
  );
  const statuses = new Map(
    testCase.records.map((record) => [
      record.id,
      deriveStatus(testCase, record, supersededIds),
    ]),
  );
  for (const [id, expectedStatus] of Object.entries(expected.status)) {
    check(
      statuses.get(id) === expectedStatus,
      `${caseId}/${id}: expected ${expectedStatus}, derived ${statuses.get(id)}`,
    );
  }
  for (const id of required) {
    check(expected.status[id] !== undefined, `${caseId}/${id}: required record needs a status assertion`);
    check(!excludedStatuses.has(statuses.get(id)), `${caseId}/${id}: required record is ${statuses.get(id)}`);
    const record = byId.get(id);
    const hasSignal =
      lexicalOverlap(testCase.query.text, record.content) > 0 ||
      (testCase.query.diagnostic_signature &&
        testCase.query.diagnostic_signature === record.diagnostic_signature);
    check(hasSignal, `${caseId}/${id}: required record has no deterministic retrieval signal`);
  }
  for (const id of forbidden) {
    check(
      excludedStatuses.has(statuses.get(id)),
      `${caseId}/${id}: forbidden record lacks a scope, lifecycle, or Git exclusion`,
    );
  }
  for (const pair of expected.rank_before) {
    check(Array.isArray(pair) && pair.length === 2, `${caseId}: rank assertion must contain two IDs`);
    const [preferredId, lowerId] = pair;
    check(preferredId !== lowerId, `${caseId}: rank assertion compares one record to itself`);
    check(byId.has(preferredId) && byId.has(lowerId), `${caseId}: rank assertion references unknown record`);
    check(
      rankingAdvantages(testCase, byId.get(preferredId), byId.get(lowerId)).length > 0,
      `${caseId}: ${preferredId} has no asserted retrieval advantage over ${lowerId}`,
    );
  }

  if (testCase.labels.includes("exact-error")) {
    check(typeof testCase.query.diagnostic_signature === "string", `${caseId}: exact-error query needs a signature`);
    check(
      [...required].some(
        (id) => byId.get(id).diagnostic_signature === testCase.query.diagnostic_signature,
      ),
      `${caseId}: exact diagnostic is not required for retrieval`,
    );
  }
}

function loadCases(path) {
  const lines = readFileSync(path, "utf8")
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0);
  check(lines.length > 0, "evaluation fixture is empty");
  return lines.map((line, index) => {
    try {
      return JSON.parse(line);
    } catch (error) {
      throw new Error(`line ${index + 1}: invalid JSON: ${error.message}`);
    }
  });
}

try {
  const cases = loadCases(fixturePath);
  const caseIds = new Set();
  const recordIds = new Set();
  for (const testCase of cases) validateCase(testCase, caseIds, recordIds);
  for (const [caseId, labels] of requiredCases) {
    check(caseIds.has(caseId), `required evaluation case is missing: ${caseId}`);
    const testCase = cases.find((candidate) => candidate.case_id === caseId);
    for (const label of labels) {
      check(testCase.labels.includes(label), `${caseId}: required label is missing: ${label}`);
    }
  }

  const requiredCount = cases.reduce(
    (total, testCase) => total + testCase.expected.must_return.length,
    0,
  );
  const forbiddenCount = cases.reduce(
    (total, testCase) => total + testCase.expected.must_not_return.length,
    0,
  );
  const rankCount = cases.reduce(
    (total, testCase) => total + testCase.expected.rank_before.length,
    0,
  );
  console.log(
    `evaluation fixture ok: ${cases.length} cases, ${recordIds.size} records, ` +
      `${requiredCount} required, ${forbiddenCount} forbidden, ${rankCount} rank assertions`,
  );
} catch (error) {
  console.error(`evaluation fixture failed: ${error.message}`);
  process.exitCode = 1;
}
