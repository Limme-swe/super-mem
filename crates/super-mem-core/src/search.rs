//! Deterministic, rebuildable search projections and vector primitives.

use std::collections::BTreeSet;

use crate::{ArtifactRef, EntityRef, Error, Result};

const MAX_ALIAS_BYTES: usize = 8_192;
const MAX_ALIAS_TERMS: usize = 512;
const MAX_OBSERVED_TERMS: usize = 4_096;
const MAX_TERM_BYTES: usize = 64;
const VECTOR_SIGNATURE_BITS: usize = 128;
const PHRASES: &[(&[&str], &[&str])] = &[
    (&["out", "of", "memory"], &["oom", "outofmemory"]),
    (&["file", "not", "found"], &["enoent", "filenotfound"]),
    (&["permission", "denied"], &["eacces", "permissiondenied"]),
    (&["connection", "refused"], &["econnrefused"]),
    (&["connection", "reset"], &["econnreset"]),
    (&["database", "locked"], &["sqlitebusy", "sqlitelocked"]),
    (&["stack", "overflow"], &["stackoverflow"]),
    (&["null", "pointer", "exception"], &["nullpointerexception"]),
    (&["unique", "constraint"], &["uniqueconstraint"]),
    (&["foreign", "key", "constraint"], &["foreignkeyconstraint"]),
];

/// Largest vector accepted by the built-in portable representation.
pub(crate) const MAX_VECTOR_DIMENSION: usize = 4_096;
/// Fixed byte width of the deterministic random-hyperplane signature.
pub(crate) const VECTOR_SIGNATURE_BYTES: usize = VECTOR_SIGNATURE_BITS / 8;
/// Persisted version of the deterministic signature algorithm.
pub(crate) const VECTOR_SIGNATURE_VERSION: u32 = 1;

/// A validated vector and its deterministic portable projections.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EncodedVector {
    /// Canonical finite `f32` components in little-endian order.
    pub(crate) float_le: Vec<u8>,
    /// LSB-first random-hyperplane bits used for the portable ANN shortlist.
    pub(crate) signature: Vec<u8>,
    /// Euclidean norm accumulated in `f64` precision.
    pub(crate) norm: f64,
}

/// A decoded, validated vector ready for exact scoring.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecodedVector {
    /// Canonical finite components. Negative zero is represented as positive zero.
    pub(crate) values: Vec<f32>,
    /// Euclidean norm accumulated in `f64` precision.
    pub(crate) norm: f64,
}

/// Builds bounded aliases for code-shaped names and conservative coding concepts.
///
/// Existing plain words are not copied. The result contains only useful alternate
/// spellings, such as the components of `MemoryEngine`, the compact form of
/// `memory_engine`, and narrow aliases for well-known error identifiers.
pub(crate) fn code_aliases(
    title: &str,
    body: &str,
    tags: &[String],
    entities: &[EntityRef],
    artifacts: &[ArtifactRef],
) -> String {
    let mut observed = BTreeSet::new();
    let mut structured = BTreeSet::new();
    let mut title_aliases = BTreeSet::new();
    let mut body_aliases = BTreeSet::new();

    let mut structured_sources = Vec::with_capacity(
        tags.len() + entities.len().saturating_mul(3) + artifacts.len().saturating_mul(2),
    );
    structured_sources.extend(tags.iter().map(String::as_str));
    for entity in entities {
        structured_sources.extend([
            entity.kind.as_str(),
            entity.canonical.as_str(),
            entity.display.as_str(),
        ]);
    }
    for artifact in artifacts {
        structured_sources.push(artifact.path.as_str());
        if let Some(symbol) = artifact.symbol.as_deref() {
            structured_sources.push(symbol);
        }
    }
    structured_sources.sort_unstable();
    structured_sources.dedup();
    for source in structured_sources {
        collect_source(source, &mut observed, &mut structured);
    }
    collect_source(title, &mut observed, &mut title_aliases);
    collect_source(body, &mut observed, &mut body_aliases);

    let mut concept_aliases = BTreeSet::new();
    add_bounded_phrase_aliases(title, &mut concept_aliases);
    add_bounded_phrase_aliases(body, &mut concept_aliases);
    add_concept_aliases(&observed, &mut concept_aliases);

    let mut output = String::new();
    let mut emitted = BTreeSet::new();
    let mut count = 0;
    for tier in [&concept_aliases, &structured, &title_aliases, &body_aliases] {
        for term in tier {
            if count == MAX_ALIAS_TERMS || emitted.contains(term) {
                continue;
            }
            let separator_bytes = usize::from(!output.is_empty());
            if output
                .len()
                .saturating_add(separator_bytes)
                .saturating_add(term.len())
                > MAX_ALIAS_BYTES
            {
                continue;
            }
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(term);
            emitted.insert(term.clone());
            count += 1;
        }
    }
    output
}

fn collect_source(source: &str, observed: &mut BTreeSet<String>, aliases: &mut BTreeSet<String>) {
    let mut unit = String::new();
    for character in source.chars() {
        if character.is_alphanumeric() || matches!(character, '_' | '-' | ':') {
            unit.push(character);
        } else {
            collect_unit(&unit, observed, aliases);
            unit.clear();
        }
        if observed.len() >= MAX_OBSERVED_TERMS {
            break;
        }
    }
    if observed.len() < MAX_OBSERVED_TERMS {
        collect_unit(&unit, observed, aliases);
    }
}

fn collect_unit(unit: &str, observed: &mut BTreeSet<String>, aliases: &mut BTreeSet<String>) {
    if unit.is_empty() {
        return;
    }

    let segments = unit
        .split(|character: char| !character.is_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return;
    }
    let separated = segments.len() > 1;

    let compact = normalize_term(&segments.concat());
    if let Some(compact) = &compact {
        observed.insert(compact.clone());
    }
    if segments.len() > 1
        && let Some(compact) = compact
    {
        aliases.insert(compact);
    }

    for segment in segments {
        let Some(normalized) = normalize_term(segment) else {
            continue;
        };
        observed.insert(normalized.clone());
        if separated {
            aliases.insert(normalized);
        }
        let components = split_camel_and_digits(segment);
        if components.len() > 1 {
            for component in components {
                if let Some(component) = normalize_term(&component) {
                    observed.insert(component.clone());
                    aliases.insert(component);
                }
            }
        }
    }
}

fn split_camel_and_digits(segment: &str) -> Vec<String> {
    let characters = segment.chars().collect::<Vec<_>>();
    if characters.is_empty() {
        return Vec::new();
    }
    let mut components = Vec::new();
    let mut start = 0;
    for index in 1..characters.len() {
        let previous = characters[index - 1];
        let current = characters[index];
        let next = characters.get(index + 1).copied();
        let digit_boundary = previous.is_numeric() != current.is_numeric()
            && (previous.is_alphabetic() || current.is_alphabetic());
        let lower_to_upper = previous.is_lowercase() && current.is_uppercase();
        let acronym_boundary = previous.is_uppercase()
            && current.is_uppercase()
            && next.is_some_and(char::is_lowercase);
        if digit_boundary || lower_to_upper || acronym_boundary {
            components.push(characters[start..index].iter().collect());
            start = index;
        }
    }
    components.push(characters[start..].iter().collect());
    components
}

fn normalize_term(term: &str) -> Option<String> {
    if !term.is_ascii() {
        return None;
    }
    let normalized = term
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let character_count = normalized.chars().count();
    if character_count < 2 || normalized.len() > MAX_TERM_BYTES {
        return None;
    }
    if normalized
        .chars()
        .all(|character| character.is_ascii_digit())
        && !(2..=5).contains(&character_count)
    {
        return None;
    }
    if normalized.len() >= 24 && normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(normalized)
}

fn add_bounded_phrase_aliases(source: &str, aliases: &mut BTreeSet<String>) {
    let mut observed_words = 0;
    for segment in source.split(['.', '!', '?', ';', '\n']) {
        let mut words = Vec::new();
        let mut current = String::new();
        for character in segment.chars() {
            if character.is_alphanumeric() {
                current.push(character);
            } else if !current.is_empty() {
                if let Some(word) = normalize_term(&current) {
                    words.push(word);
                    observed_words += 1;
                }
                current.clear();
            }
            if observed_words == MAX_OBSERVED_TERMS {
                break;
            }
        }
        if observed_words < MAX_OBSERVED_TERMS
            && !current.is_empty()
            && let Some(word) = normalize_term(&current)
        {
            words.push(word);
            observed_words += 1;
        }
        for &(phrase, expansions) in PHRASES {
            if contains_bounded_phrase(&words, phrase) {
                aliases.extend(expansions.iter().map(|expansion| (*expansion).to_owned()));
            }
        }
        if observed_words == MAX_OBSERVED_TERMS {
            break;
        }
    }
}

fn contains_bounded_phrase(words: &[String], phrase: &[&str]) -> bool {
    if phrase.is_empty() {
        return false;
    }
    'start: for start in 0..words.len() {
        if words[start] != phrase[0] {
            continue;
        }
        let mut cursor = start;
        for expected in &phrase[1..] {
            let next = cursor + 1;
            if next >= words.len() {
                continue 'start;
            }
            let end = (cursor + 2).min(words.len() - 1);
            let Some(offset) = words[next..=end].iter().position(|word| word == expected) else {
                continue 'start;
            };
            cursor = next + offset;
        }
        return true;
    }
    false
}

fn add_concept_aliases(observed: &BTreeSet<String>, aliases: &mut BTreeSet<String>) {
    const CONCEPTS: &[(&[&str], &[&str])] = &[
        (&["cfg"], &["config", "configuration"]),
        (&["repo"], &["repository"]),
        (&["deps"], &["dependency", "dependencies"]),
        (&["cli"], &["command", "line", "interface"]),
        (&["db", "database"], &["db", "database"]),
        (&["fs"], &["filesystem"]),
        (&["authn"], &["authentication"]),
        (&["authz"], &["authorization"]),
        (
            &["oom", "outofmemory", "outofmemoryerror"],
            &["memoryexhaustion", "allocationfailure"],
        ),
        (
            &["sigsegv", "segfault", "segmentationfault"],
            &["segmentationfault", "invalidmemoryaccess"],
        ),
        (&["enoent"], &["filenotfound", "missingpath"]),
        (&["eacces", "eperm"], &["permissiondenied", "accessdenied"]),
        (
            &["etimedout", "timedout", "deadlineexceeded"],
            &["timeout", "deadlineexceeded"],
        ),
        (&["econnrefused"], &["connectionrefused"]),
        (&["econnreset"], &["connectionreset"]),
        (&["eaddrinuse"], &["addressinuse", "occupied", "portinuse"]),
        (
            &["sqlitebusy", "sqlitelocked"],
            &["contention", "database", "databaselocked", "lockcontention"],
        ),
        (&["deadlock"], &["lockcontention"]),
        (
            &["await", "awaited", "awaiting", "awaits"],
            &["suspend", "suspension"],
        ),
        (&["spawn", "spawned", "spawning"], &["launch", "task"]),
        (
            &["sqlxoffline"],
            &["compilewithoutdatabase", "database", "offlinebuild"],
        ),
        (&["bounded"], &["backpressure", "memorygrowth", "ram"]),
        (
            &["duplicatekey", "uniqueconstraint"],
            &["uniquenessviolation"],
        ),
        (&["foreignkeyconstraint"], &["referentialintegrity"]),
        (
            &[
                "nullpointerexception",
                "nilpointer",
                "nullreferenceexception",
            ],
            &["nullreference", "nilpointer"],
        ),
        (&["useafterfree"], &["danglingpointer", "memorysafety"]),
        (
            &["bufferoverflow", "indexoutofbounds", "outofbounds"],
            &["boundscheck", "outofbounds"],
        ),
        (&["stackoverflow"], &["callstack", "recursionlimit"]),
        (&["http401"], &["unauthorized", "authentication"]),
        (&["http403"], &["forbidden", "authorization"]),
        (&["http404"], &["notfound"]),
        (&["http429"], &["ratelimit", "throttled"]),
    ];

    for (triggers, expansions) in CONCEPTS {
        if triggers.iter().any(|trigger| observed.contains(*trigger)) {
            add_unobserved_expansions(observed, aliases, expansions);
        }
    }

    // Phrase aliases can introduce a machine identifier (for example,
    // "database locked" -> `sqlitebusy`). Expand that identifier once so its
    // human-facing concepts are indexed too without an unbounded closure.
    for (triggers, expansions) in CONCEPTS {
        if triggers.iter().any(|trigger| aliases.contains(*trigger)) {
            add_unobserved_expansions(observed, aliases, expansions);
        }
    }
}

fn add_unobserved_expansions(
    observed: &BTreeSet<String>,
    aliases: &mut BTreeSet<String>,
    expansions: &[&str],
) {
    for &expansion in expansions {
        if !observed.contains(expansion) {
            aliases.insert(expansion.to_owned());
        }
    }
}

/// Encodes a finite, non-zero vector and its random-hyperplane signature.
pub(crate) fn encode_f32_vector(values: &[f32], expected_dim: usize) -> Result<EncodedVector> {
    validate_dimension(values.len(), expected_dim)?;
    let mut float_le = Vec::with_capacity(expected_dim.saturating_mul(4));
    let mut canonical_values = Vec::with_capacity(expected_dim);
    let mut norm_squared = 0.0_f64;

    for (index, &value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(Error::InvalidInput(format!(
                "vector component {index} is not finite"
            )));
        }
        let canonical = if value.to_bits() == (-0.0_f32).to_bits() {
            0.0
        } else {
            value
        };
        float_le.extend_from_slice(&canonical.to_le_bytes());
        canonical_values.push(canonical);
        let widened = f64::from(canonical);
        norm_squared += widened * widened;
    }
    if norm_squared.to_bits() == 0.0_f64.to_bits() {
        return Err(Error::InvalidInput(
            "a search vector must not have zero norm".into(),
        ));
    }
    Ok(EncodedVector {
        float_le,
        signature: random_hyperplane_signature(&canonical_values),
        norm: norm_squared.sqrt(),
    })
}

/// Decodes and validates a canonical little-endian `f32` vector.
pub(crate) fn decode_f32_vector(float_le: &[u8], expected_dim: usize) -> Result<DecodedVector> {
    validate_expected_dimension(expected_dim)?;
    let expected_bytes = expected_dim
        .checked_mul(4)
        .ok_or_else(|| Error::InvalidInput("vector byte length overflowed".into()))?;
    if float_le.len() != expected_bytes {
        return Err(Error::InvalidInput(format!(
            "vector blob has {} bytes; expected {expected_bytes}",
            float_le.len()
        )));
    }

    let mut values = Vec::with_capacity(expected_dim);
    let mut norm_squared = 0.0_f64;
    for (index, chunk) in float_le.chunks_exact(4).enumerate() {
        let raw: [u8; 4] = chunk
            .try_into()
            .map_err(|_| Error::InvalidInput("invalid vector component width".into()))?;
        let value = f32::from_le_bytes(raw);
        if !value.is_finite() {
            return Err(Error::InvalidInput(format!(
                "vector component {index} is not finite"
            )));
        }
        let canonical = if value.to_bits() == (-0.0_f32).to_bits() {
            0.0
        } else {
            value
        };
        let widened = f64::from(canonical);
        norm_squared += widened * widened;
        values.push(canonical);
    }
    if norm_squared.to_bits() == 0.0_f64.to_bits() {
        return Err(Error::InvalidInput(
            "a search vector must not have zero norm".into(),
        ));
    }
    Ok(DecodedVector {
        values,
        norm: norm_squared.sqrt(),
    })
}

/// Validates the fixed width of a stored random-hyperplane signature.
pub(crate) fn validate_signature_width(signature: &[u8]) -> Result<()> {
    if signature.len() == VECTOR_SIGNATURE_BYTES {
        return Ok(());
    }
    Err(Error::InvalidInput(format!(
        "vector signature has {} bytes; expected {VECTOR_SIGNATURE_BYTES}",
        signature.len()
    )))
}

#[cfg(test)]
fn validate_signature_matches(signature: &[u8], vector: &DecodedVector) -> Result<()> {
    validate_decoded_vector(vector)?;
    validate_signature_width(signature)?;
    if signature != random_hyperplane_signature(&vector.values) {
        return Err(Error::InvalidInput(
            "vector signature does not match its components".into(),
        ));
    }
    Ok(())
}

/// Produces a deterministic SimHash-style angular signature.
///
/// Each bit is the sign of a fixed Rademacher random hyperplane. Unlike raw
/// coordinate signs, Hamming distance over these bits estimates angular
/// distance even when two vectors occupy the same coordinate orthant.
fn random_hyperplane_signature(values: &[f32]) -> Vec<u8> {
    let mut signature = vec![0_u8; VECTOR_SIGNATURE_BYTES];
    let dimension = values.len() as u64;
    for bit in 0..VECTOR_SIGNATURE_BITS {
        let mut dot = 0.0_f64;
        for (index, &value) in values.iter().enumerate() {
            let random = mix64(
                0x6a09_e667_f3bc_c909_u64
                    ^ dimension.wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    ^ (bit as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
                    ^ (index as u64).wrapping_mul(0x94d0_49bb_1331_11eb),
            );
            if random & 1 == 0 {
                dot += f64::from(value);
            } else {
                dot -= f64::from(value);
            }
        }
        if dot < 0.0 {
            signature[bit / 8] |= 1 << (bit % 8);
        }
    }
    signature
}

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Computes exact cosine similarity for two validated vectors.
pub(crate) fn cosine_similarity(left: &DecodedVector, right: &DecodedVector) -> Result<f64> {
    if left.values.len() != right.values.len() {
        return Err(Error::InvalidInput(format!(
            "cosine vectors have different dimensions: {} and {}",
            left.values.len(),
            right.values.len()
        )));
    }
    let left_norm = validate_decoded_vector(left)?;
    let right_norm = validate_decoded_vector(right)?;
    let dot = left
        .values
        .iter()
        .zip(&right.values)
        .fold(0.0_f64, |sum, (&left, &right)| {
            sum + f64::from(left) * f64::from(right)
        });
    Ok((dot / (left_norm * right_norm)).clamp(-1.0, 1.0))
}

fn validate_decoded_vector(vector: &DecodedVector) -> Result<f64> {
    validate_expected_dimension(vector.values.len())?;
    if !vector.norm.is_finite() || vector.norm <= 0.0 {
        return Err(Error::InvalidInput(
            "decoded vector has an invalid norm".into(),
        ));
    }
    let mut norm_squared = 0.0_f64;
    for (index, &value) in vector.values.iter().enumerate() {
        if !value.is_finite() {
            return Err(Error::InvalidInput(format!(
                "vector component {index} is not finite"
            )));
        }
        if value.to_bits() == (-0.0_f32).to_bits() {
            return Err(Error::InvalidInput(format!(
                "vector component {index} is negative zero"
            )));
        }
        let widened = f64::from(value);
        norm_squared += widened * widened;
    }
    let computed_norm = norm_squared.sqrt();
    let tolerance = computed_norm.max(vector.norm).max(1.0) * f64::EPSILON * 8.0;
    if (vector.norm - computed_norm).abs() > tolerance {
        return Err(Error::InvalidInput(
            "decoded vector norm does not match its components".into(),
        ));
    }
    Ok(computed_norm)
}

/// Sorts `(identity, cosine)` pairs by descending score and then identity.
pub(crate) fn rank_by_cosine<T: Ord>(scores: &mut [(T, f64)]) -> Result<()> {
    if scores.iter().any(|(_, score)| !score.is_finite()) {
        return Err(Error::InvalidInput(
            "cosine ranking contains a non-finite score".into(),
        ));
    }
    scores.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    Ok(())
}

/// Counts differing bits in equal-length, bounded binary vector projections.
pub(crate) fn hamming_distance(left: &[u8], right: &[u8]) -> Result<u32> {
    if left.len() != right.len() {
        return Err(Error::InvalidInput(format!(
            "binary vectors have different lengths: {} and {}",
            left.len(),
            right.len()
        )));
    }
    if left.len() > VECTOR_SIGNATURE_BYTES {
        return Err(Error::InvalidInput(
            "binary signature exceeds the supported width".into(),
        ));
    }
    Ok(left
        .iter()
        .zip(right)
        .map(|(&left, &right)| (left ^ right).count_ones())
        .sum())
}

fn validate_dimension(actual_dim: usize, expected_dim: usize) -> Result<()> {
    validate_expected_dimension(expected_dim)?;
    if actual_dim != expected_dim {
        return Err(Error::InvalidInput(format!(
            "vector has dimension {actual_dim}; expected {expected_dim}"
        )));
    }
    Ok(())
}

fn validate_expected_dimension(expected_dim: usize) -> Result<()> {
    if expected_dim == 0 || expected_dim > MAX_VECTOR_DIMENSION {
        return Err(Error::InvalidInput(format!(
            "vector dimension must be between 1 and {MAX_VECTOR_DIMENSION}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(canonical: &str, display: &str) -> EntityRef {
        EntityRef {
            kind: "symbol".into(),
            canonical: canonical.into(),
            display: display.into(),
        }
    }

    fn artifact(path: &str, symbol: &str) -> ArtifactRef {
        ArtifactRef {
            path: path.into(),
            symbol: Some(symbol.into()),
            ..ArtifactRef::default()
        }
    }

    #[test]
    fn aliases_split_code_shapes_and_add_narrow_error_concepts() {
        let aliases = code_aliases(
            "MemoryEngine handles HTTPServer20XX",
            "SQLite_BUSY followed EACCES in use-after-free cleanup",
            &["cli".into(), "memory_store".into()],
            &[entity("parse_http_response", "ParseHTTPResponse")],
            &[artifact(
                "src/http-client/v2/parser.rs",
                "registerArtifact20D",
            )],
        );
        let terms = aliases.split_whitespace().collect::<BTreeSet<_>>();

        for required in [
            "memory",
            "engine",
            "http",
            "server",
            "20",
            "xx",
            "memorystore",
            "parsehttpresponse",
            "httpclient",
            "register",
            "artifact",
            "databaselocked",
            "lockcontention",
            "permissiondenied",
            "danglingpointer",
            "memorysafety",
            "command",
            "line",
            "interface",
        ] {
            assert!(
                terms.contains(required),
                "missing alias {required}: {aliases}"
            );
        }
        assert!(!terms.contains("handles"));
        assert_eq!(terms.len(), aliases.split_whitespace().count());
    }

    #[test]
    fn aliases_are_order_independent_and_bounded() {
        let first = code_aliases(
            "AlphaHandler BetaParser",
            &repeated_code_body(),
            &["repo".into(), "cfg".into()],
            &[
                entity("zeta_handler", "ZetaHandler"),
                entity("alpha_parser", "AlphaParser"),
            ],
            &[
                artifact("src/zeta-file.rs", "ZetaWriter"),
                artifact("src/alpha-file.rs", "AlphaReader"),
            ],
        );
        let second = code_aliases(
            "AlphaHandler BetaParser",
            &repeated_code_body(),
            &["cfg".into(), "repo".into()],
            &[
                entity("alpha_parser", "AlphaParser"),
                entity("zeta_handler", "ZetaHandler"),
            ],
            &[
                artifact("src/alpha-file.rs", "AlphaReader"),
                artifact("src/zeta-file.rs", "ZetaWriter"),
            ],
        );
        assert_eq!(first, second);
        assert!(first.len() <= MAX_ALIAS_BYTES);
        assert!(first.split_whitespace().count() <= MAX_ALIAS_TERMS);
    }

    #[test]
    fn aliases_map_only_complete_error_phrases_back_to_codes() {
        let aliases = code_aliases(
            "Recovery failures",
            "The file was not found and the database was locked",
            &[],
            &[],
            &[],
        );
        let terms = aliases.split_whitespace().collect::<BTreeSet<_>>();
        assert!(terms.contains("enoent"));
        assert!(terms.contains("sqlitebusy"));

        let partial = code_aliases("Missing", "The result was not found", &[], &[], &[]);
        assert!(!partial.split_whitespace().any(|term| term == "enoent"));

        let distant = code_aliases(
            "Database recovery",
            "The database pool reopened. A worker later locked its own mutex.",
            &[],
            &[],
            &[],
        );
        assert!(!distant.split_whitespace().any(|term| term == "sqlitebusy"));
    }

    #[test]
    fn aliases_cover_common_runtime_and_backpressure_wording() {
        let aliases = code_aliases(
            "Bounded worker startup",
            "A spawned future awaits capacity; EADDRINUSE was raised by SQLX_OFFLINE checks",
            &[],
            &[],
            &[],
        );
        let terms = aliases.split_whitespace().collect::<BTreeSet<_>>();
        for required in [
            "backpressure",
            "database",
            "launch",
            "occupied",
            "ram",
            "suspend",
        ] {
            assert!(
                terms.contains(required),
                "missing alias {required}: {aliases}"
            );
        }
    }

    #[test]
    fn aliases_split_repository_email_symbol_components() {
        let aliases = code_aliases(
            "Account address lookup",
            "UserRepository::find_by_email returns the matching account.",
            &[],
            &[entity(
                "userrepository::find_by_email",
                "UserRepository::find_by_email",
            )],
            &[artifact(
                "src/users/repository.rs",
                "UserRepository::find_by_email",
            )],
        );
        let terms = aliases.split_whitespace().collect::<BTreeSet<_>>();
        for required in ["user", "repository", "find", "by", "email"] {
            assert!(
                terms.contains(required),
                "missing alias {required}: {aliases}"
            );
        }
    }

    fn repeated_code_body() -> String {
        let mut body = String::new();
        for index in 0..2_000 {
            body.push_str("Thing");
            body.push_str(&index.to_string());
            body.push_str("Handler ");
        }
        body
    }

    #[test]
    fn vector_encoding_is_canonical_and_round_trips() {
        let encoded = encode_f32_vector(&[1.0, -0.0, -2.0, 3.5], 4).unwrap();
        assert_eq!(&encoded.float_le[4..8], &0.0_f32.to_le_bytes());
        assert_eq!(encoded.signature.len(), VECTOR_SIGNATURE_BYTES);

        let decoded = decode_f32_vector(&encoded.float_le, 4).unwrap();
        assert_eq!(decoded.values, vec![1.0, 0.0, -2.0, 3.5]);
        assert_eq!(decoded.norm.to_bits(), encoded.norm.to_bits());
        validate_signature_matches(&encoded.signature, &decoded).unwrap();
    }

    #[test]
    fn vector_validation_rejects_invalid_shapes_and_values() {
        assert!(encode_f32_vector(&[1.0], 2).is_err());
        assert!(encode_f32_vector(&[], 0).is_err());
        assert!(encode_f32_vector(&[0.0, -0.0], 2).is_err());
        assert!(encode_f32_vector(&[f32::NAN], 1).is_err());
        assert!(encode_f32_vector(&[f32::INFINITY], 1).is_err());
        assert!(decode_f32_vector(&[0; 3], 1).is_err());

        let encoded = encode_f32_vector(&[1.0, -2.0, 3.0], 3).unwrap();
        let decoded = decode_f32_vector(&encoded.float_le, 3).unwrap();
        assert!(validate_signature_matches(&[0], &decoded).is_err());
        let mut invalid_signature = encoded.signature.clone();
        invalid_signature[0] ^= 1;
        assert!(validate_signature_matches(&invalid_signature, &decoded).is_err());

        let mut wrong_norm = decoded.clone();
        wrong_norm.norm *= 2.0;
        assert!(cosine_similarity(&decoded, &wrong_norm).is_err());

        let negative_zero = DecodedVector {
            values: vec![-0.0, 1.0, 2.0],
            norm: 5.0_f64.sqrt(),
        };
        assert!(validate_signature_matches(&[0; VECTOR_SIGNATURE_BYTES], &negative_zero).is_err());
    }

    #[test]
    fn random_hyperplane_signature_tracks_angular_distance() {
        let first = encode_f32_vector(&[1.0, 2.0, 3.0, 5.0], 4).unwrap();
        let same_direction = encode_f32_vector(&[2.0, 4.0, 6.0, 10.0], 4).unwrap();
        let opposite = encode_f32_vector(&[-1.0, -2.0, -3.0, -5.0], 4).unwrap();

        assert_eq!(
            hamming_distance(&first.signature, &same_direction.signature).unwrap(),
            0
        );
        assert_eq!(
            hamming_distance(&first.signature, &opposite.signature).unwrap(),
            VECTOR_SIGNATURE_BITS as u32
        );
    }

    #[test]
    fn cosine_and_ranking_are_exact_and_totally_ordered() {
        let query_blob = encode_f32_vector(&[1.0, 0.0], 2).unwrap();
        let same_blob = encode_f32_vector(&[2.0, 0.0], 2).unwrap();
        let orthogonal_blob = encode_f32_vector(&[0.0, 4.0], 2).unwrap();
        let opposite_blob = encode_f32_vector(&[-1.0, 0.0], 2).unwrap();
        let query = decode_f32_vector(&query_blob.float_le, 2).unwrap();
        let same = decode_f32_vector(&same_blob.float_le, 2).unwrap();
        let orthogonal = decode_f32_vector(&orthogonal_blob.float_le, 2).unwrap();
        let opposite = decode_f32_vector(&opposite_blob.float_le, 2).unwrap();

        assert_eq!(
            cosine_similarity(&query, &same).unwrap().to_bits(),
            1.0_f64.to_bits()
        );
        assert_eq!(
            cosine_similarity(&query, &orthogonal).unwrap().to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(
            cosine_similarity(&query, &opposite).unwrap().to_bits(),
            (-1.0_f64).to_bits()
        );

        let mut scores = vec![("z", 0.5), ("b", 1.0), ("a", 0.5)];
        rank_by_cosine(&mut scores).unwrap();
        assert_eq!(scores, vec![("b", 1.0), ("a", 0.5), ("z", 0.5)]);
        assert!(rank_by_cosine(&mut [("bad", f64::NAN)]).is_err());
    }

    #[test]
    fn hamming_distance_counts_bits_and_validates_width() {
        assert_eq!(hamming_distance(&[0b1010_0000], &[0b0011_0000]).unwrap(), 2);
        assert!(hamming_distance(&[0], &[0, 1]).is_err());
    }
}
