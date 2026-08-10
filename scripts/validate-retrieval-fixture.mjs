#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const fixtureDirectory = fileURLToPath(
  new URL("../fixtures/retrieval/", import.meta.url),
);
const casePath = resolve(process.argv[2] ?? fixtureDirectory, "v1.jsonl");
const qrelPath = resolve(process.argv[2] ?? fixtureDirectory, "qrels-v1.jsonl");

const caseSchema = "supermem.retrieval.case.v1";
const qrelSchema = "supermem.retrieval.qrels.v1";
const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const hexOidPattern = /^[0-9a-f]{7,64}$/;
const memoryKinds = new Set([
  "fact",
  "preference",
  "constraint",
  "decision",
  "procedure",
  "episode",
  "outcome",
  "task",
  "observation",
]);
const trustLevels = new Set([
  "external",
  "agent",
  "tool_verified",
  "user_confirmed",
]);
const retrievalSignals = new Set([
  "exact",
  "lexical",
  "lexical_strict",
  "code_alias_strict",
  "code_alias",
  "semantic_expansion",
  "dense_vector",
  "sparse",
  "entity",
  "recency",
  "artifact_verified",
  "error_fingerprint",
]);
const applicabilityValues = new Set([
  "exact",
  "compatible",
  "stale",
  "divergent",
  "unversioned",
  "inapplicable",
]);
const caseFields = new Set([
  "schema_version",
  "case_id",
  "operations",
  "recall",
]);
const operationFields = new Set(["op", "record_id", "request"]);
const scopeFields = new Set([
  "namespace",
  "workspace_id",
  "repository",
  "session_id",
]);
const repositoryFields = new Set([
  "repo_id",
  "root",
  "common_dir",
  "branch",
  "head_oid",
  "remote",
  "dirty_hash",
]);
const artifactFields = new Set([
  "repo_id",
  "path",
  "symbol",
  "content_hash",
  "git_oid",
  "language",
]);
const rememberFields = new Set([
  "memory_id",
  "idempotency_key",
  "kind",
  "scope",
  "canonical_key",
  "title",
  "body",
  "importance",
  "confidence",
  "trust",
  "valid_from",
  "valid_until",
  "expires_at",
  "attributes",
  "tags",
  "entities",
  "artifacts",
  "evidence",
  "links",
]);
const recallFields = new Set([
  "query",
  "scope",
  "limit",
  "token_budget",
  "kinds",
  "as_of",
  "include_stale",
  "include_divergent",
  "include_superseded",
  "hints",
]);
const hintFields = new Set([
  "artifacts",
  "error_fingerprint",
  "entities",
  "dense",
]);
const qrelFields = new Set([
  "schema_version",
  "case_id",
  "relevance",
  "forbidden",
  "rank_before",
  "expected_signals",
  "expected_applicability",
  "expected_excluded",
  "expected_revision",
  "forbidden_body_substrings",
]);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function exactFields(value, allowed, context) {
  for (const field of Object.keys(value)) {
    check(allowed.has(field), `${context}: unexpected field ${field}`);
  }
}

function nonEmptyString(value, context) {
  check(typeof value === "string" && value.trim().length > 0, `${context} is required`);
}

function uniqueStrings(values, context) {
  check(Array.isArray(values), `${context} must be an array`);
  check(
    values.every((value) => typeof value === "string" && value.length > 0),
    `${context} must contain non-empty strings`,
  );
  check(new Set(values).size === values.length, `${context} contains duplicates`);
}

function validateScope(scope, context) {
  check(object(scope), `${context} must be an object`);
  exactFields(scope, scopeFields, context);
  nonEmptyString(scope.namespace, `${context}.namespace`);
  if (scope.workspace_id !== undefined && scope.workspace_id !== null) {
    nonEmptyString(scope.workspace_id, `${context}.workspace_id`);
  }
  if (scope.repository !== undefined && scope.repository !== null) {
    check(object(scope.repository), `${context}.repository must be an object`);
    exactFields(scope.repository, repositoryFields, `${context}.repository`);
    nonEmptyString(scope.repository.repo_id, `${context}.repository.repo_id`);
    if (scope.repository.head_oid !== undefined && scope.repository.head_oid !== null) {
      check(
        hexOidPattern.test(scope.repository.head_oid),
        `${context}.repository.head_oid is invalid`,
      );
    }
  }
}

function validateArtifacts(artifacts, request, context) {
  check(Array.isArray(artifacts), `${context} must be an array`);
  for (const [index, artifact] of artifacts.entries()) {
    const item = `${context}[${index}]`;
    check(object(artifact), `${item} must be an object`);
    exactFields(artifact, artifactFields, item);
    nonEmptyString(artifact.repo_id, `${item}.repo_id`);
    nonEmptyString(artifact.path, `${item}.path`);
    check(!artifact.path.startsWith("/"), `${item}.path must be repository-relative`);
    check(!artifact.path.split(/[\\/]/).includes(".."), `${item}.path contains traversal`);
    const scopedRepo = request.scope.repository?.repo_id;
    check(
      scopedRepo === undefined || artifact.repo_id === scopedRepo,
      `${item} crosses the request repository boundary`,
    );
  }
}

function validateRemember(operation, caseId, recordIds, revisionCounts) {
  check(object(operation), `${caseId}: every operation must be an object`);
  exactFields(operation, operationFields, `${caseId}: operation`);
  check(operation.op === "remember", `${caseId}: unsupported operation ${operation.op}`);
  nonEmptyString(operation.record_id, `${caseId}: operation.record_id`);
  check(!recordIds.has(operation.record_id), `${caseId}: duplicate record_id ${operation.record_id}`);
  recordIds.add(operation.record_id);

  const request = operation.request;
  check(object(request), `${caseId}/${operation.record_id}: request must be an object`);
  exactFields(request, rememberFields, `${caseId}/${operation.record_id}: request`);
  check(uuidPattern.test(request.memory_id), `${caseId}/${operation.record_id}: invalid memory_id`);
  check(memoryKinds.has(request.kind), `${caseId}/${operation.record_id}: invalid memory kind`);
  check(trustLevels.has(request.trust), `${caseId}/${operation.record_id}: invalid trust`);
  validateScope(request.scope, `${caseId}/${operation.record_id}: scope`);
  nonEmptyString(request.title, `${caseId}/${operation.record_id}: title`);
  nonEmptyString(request.body, `${caseId}/${operation.record_id}: body`);
  for (const field of ["importance", "confidence"]) {
    check(
      typeof request[field] === "number" && request[field] >= 0 && request[field] <= 1,
      `${caseId}/${operation.record_id}: ${field} must be in [0,1]`,
    );
  }
  check(object(request.attributes), `${caseId}/${operation.record_id}: attributes must be an object`);
  uniqueStrings(request.tags, `${caseId}/${operation.record_id}: tags`);
  check(Array.isArray(request.entities), `${caseId}/${operation.record_id}: entities must be an array`);
  validateArtifacts(request.artifacts, request, `${caseId}/${operation.record_id}: artifacts`);
  check(Array.isArray(request.evidence), `${caseId}/${operation.record_id}: evidence must be an array`);
  check(Array.isArray(request.links), `${caseId}/${operation.record_id}: links must be an array`);
  revisionCounts.set(request.memory_id, (revisionCounts.get(request.memory_id) ?? 0) + 1);
}

function validateRecall(recall, caseId) {
  check(object(recall), `${caseId}: recall must be an object`);
  exactFields(recall, recallFields, `${caseId}: recall`);
  nonEmptyString(recall.query, `${caseId}: recall.query`);
  validateScope(recall.scope, `${caseId}: recall.scope`);
  check(Number.isInteger(recall.limit) && recall.limit > 0, `${caseId}: invalid recall.limit`);
  check(
    Number.isInteger(recall.token_budget) && recall.token_budget >= 64,
    `${caseId}: invalid recall.token_budget`,
  );
  check(Array.isArray(recall.kinds), `${caseId}: recall.kinds must be an array`);
  check(recall.kinds.every((kind) => memoryKinds.has(kind)), `${caseId}: invalid recall kind`);
  check(
    typeof recall.as_of === "string" && Number.isFinite(Date.parse(recall.as_of)),
    `${caseId}: invalid recall.as_of`,
  );
  for (const field of ["include_stale", "include_divergent", "include_superseded"]) {
    check(typeof recall[field] === "boolean", `${caseId}: ${field} must be boolean`);
  }
  check(object(recall.hints), `${caseId}: recall.hints must be an object`);
  exactFields(recall.hints, hintFields, `${caseId}: recall.hints`);
  check(Array.isArray(recall.hints.artifacts), `${caseId}: recall hint artifacts must be an array`);
  check(Array.isArray(recall.hints.entities), `${caseId}: recall hint entities must be an array`);
  check(
    recall.hints.error_fingerprint === null ||
      typeof recall.hints.error_fingerprint === "string",
    `${caseId}: invalid error fingerprint`,
  );
  if (recall.hints.dense !== undefined && recall.hints.dense !== null) {
    check(object(recall.hints.dense), `${caseId}: dense hint must be an object`);
    nonEmptyString(recall.hints.dense.profile_id, `${caseId}: dense profile_id`);
    check(
      Array.isArray(recall.hints.dense.vector) &&
        recall.hints.dense.vector.length > 0 &&
        recall.hints.dense.vector.every(Number.isFinite),
      `${caseId}: dense vector must be finite and non-empty`,
    );
  }
  validateArtifacts(
    recall.hints.artifacts,
    { scope: recall.scope },
    `${caseId}: recall.hints.artifacts`,
  );
}

function loadJsonl(path) {
  return readFileSync(path, "utf8")
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0)
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        throw new Error(`${path}:${index + 1}: ${error.message}`);
      }
    });
}

function validateCases() {
  const cases = loadJsonl(casePath);
  check(cases.length >= 12, "retrieval fixture must retain at least its 12 starter cases");
  const caseIds = new Set();
  const casesById = new Map();
  for (const testCase of cases) {
    check(object(testCase), "every retrieval case must be an object");
    exactFields(testCase, caseFields, "retrieval case");
    check(testCase.schema_version === caseSchema, `${testCase.case_id}: unsupported case schema`);
    nonEmptyString(testCase.case_id, "case_id");
    check(!caseIds.has(testCase.case_id), `duplicate case_id ${testCase.case_id}`);
    caseIds.add(testCase.case_id);
    check(
      Array.isArray(testCase.operations) && testCase.operations.length > 0,
      `${testCase.case_id}: operations are required`,
    );
    const recordIds = new Set();
    const revisionCounts = new Map();
    for (const operation of testCase.operations) {
      validateRemember(operation, testCase.case_id, recordIds, revisionCounts);
    }
    validateRecall(testCase.recall, testCase.case_id);
    const memoryIds = new Set(testCase.operations.map((operation) => operation.request.memory_id));
    casesById.set(testCase.case_id, { testCase, memoryIds, revisionCounts });
  }
  return casesById;
}

function validateIdMap(values, context, memoryIds, validateValue) {
  check(object(values), `${context} must be an object`);
  for (const [memoryId, value] of Object.entries(values)) {
    check(memoryIds.has(memoryId), `${context} references unknown memory ${memoryId}`);
    validateValue(value, `${context}.${memoryId}`);
  }
}

function validateQrels(casesById) {
  const qrels = loadJsonl(qrelPath);
  check(qrels.length === casesById.size, "every retrieval case needs exactly one qrel row");
  const seen = new Set();
  let rankAssertions = 0;
  let forbiddenAssertions = 0;
  for (const qrel of qrels) {
    check(object(qrel), "every qrel row must be an object");
    exactFields(qrel, qrelFields, "qrel row");
    check(qrel.schema_version === qrelSchema, `${qrel.case_id}: unsupported qrel schema`);
    nonEmptyString(qrel.case_id, "qrel case_id");
    check(!seen.has(qrel.case_id), `duplicate qrels for ${qrel.case_id}`);
    seen.add(qrel.case_id);
    const fixture = casesById.get(qrel.case_id);
    check(fixture !== undefined, `qrels reference missing case ${qrel.case_id}`);
    const { memoryIds, revisionCounts } = fixture;

    validateIdMap(qrel.relevance, `${qrel.case_id}: relevance`, memoryIds, (value, context) => {
      check(Number.isInteger(value) && value >= 0 && value <= 3, `${context} must be in [0,3]`);
    });
    check(
      Object.values(qrel.relevance).some((grade) => grade === 3),
      `${qrel.case_id}: at least one essential result is required`,
    );
    check(
      Object.keys(qrel.relevance).length === memoryIds.size,
      `${qrel.case_id}: every logical memory needs an explicit relevance grade`,
    );

    uniqueStrings(qrel.forbidden, `${qrel.case_id}: forbidden`);
    for (const memoryId of qrel.forbidden) {
      check(memoryIds.has(memoryId), `${qrel.case_id}: forbidden unknown memory ${memoryId}`);
      check(qrel.relevance[memoryId] === 0, `${qrel.case_id}: forbidden memory must have grade 0`);
    }
    forbiddenAssertions += qrel.forbidden.length;

    check(Array.isArray(qrel.rank_before), `${qrel.case_id}: rank_before must be an array`);
    for (const pair of qrel.rank_before) {
      check(Array.isArray(pair) && pair.length === 2, `${qrel.case_id}: invalid rank pair`);
      const [preferred, lower] = pair;
      check(memoryIds.has(preferred) && memoryIds.has(lower), `${qrel.case_id}: rank pair references unknown memory`);
      check(
        qrel.relevance[preferred] > qrel.relevance[lower],
        `${qrel.case_id}: rank pair must prefer a higher relevance grade`,
      );
      rankAssertions += 1;
    }

    validateIdMap(
      qrel.expected_signals ?? {},
      `${qrel.case_id}: expected_signals`,
      memoryIds,
      (signals, context) => {
        uniqueStrings(signals, context);
        check(signals.every((signal) => retrievalSignals.has(signal)), `${context} has an invalid signal`);
        const memoryId = context.slice(context.lastIndexOf(".") + 1);
        check(qrel.relevance[memoryId] > 0, `${context} must describe a relevant memory`);
      },
    );
    for (const field of ["expected_applicability", "expected_excluded"]) {
      validateIdMap(qrel[field] ?? {}, `${qrel.case_id}: ${field}`, memoryIds, (value, context) => {
        check(applicabilityValues.has(value), `${context} has an invalid applicability`);
        const memoryId = context.slice(context.lastIndexOf(".") + 1);
        if (field === "expected_applicability") {
          check(qrel.relevance[memoryId] > 0, `${context} must describe a relevant memory`);
        } else {
          check(qrel.forbidden.includes(memoryId), `${context} must describe a forbidden memory`);
        }
      });
    }
    validateIdMap(
      qrel.expected_revision ?? {},
      `${qrel.case_id}: expected_revision`,
      memoryIds,
      (value, context) => {
        const memoryId = context.slice(context.lastIndexOf(".") + 1);
        check(Number.isInteger(value) && value > 0, `${context} must be positive`);
        check(value === revisionCounts.get(memoryId), `${context} does not name the current revision`);
        check(qrel.relevance[memoryId] > 0, `${context} must describe a relevant memory`);
      },
    );
    uniqueStrings(
      qrel.forbidden_body_substrings ?? [],
      `${qrel.case_id}: forbidden_body_substrings`,
    );
  }
  for (const caseId of casesById.keys()) {
    check(seen.has(caseId), `missing qrels for ${caseId}`);
  }
  return { cases: qrels.length, rankAssertions, forbiddenAssertions };
}

try {
  const casesById = validateCases();
  const summary = validateQrels(casesById);
  const writes = [...casesById.values()].reduce(
    (total, fixture) => total + fixture.testCase.operations.length,
    0,
  );
  console.log(
    `retrieval fixture ok: ${summary.cases} cases, ${writes} writes, ` +
      `${summary.forbiddenAssertions} forbidden, ${summary.rankAssertions} gated rank assertions`,
  );
} catch (error) {
  console.error(`retrieval fixture failed: ${error.message}`);
  process.exitCode = 1;
}
