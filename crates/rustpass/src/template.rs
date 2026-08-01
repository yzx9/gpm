// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! gopass-compatible secret templates and create presets.
//!
//! Two concepts, both mirroring gopass:
//!
//! - **Content templates** (`.pass-template`): a file named `.pass-template`
//!   placed in a directory of the store applies to every secret created beneath
//!   it. [`lookup_template_in_repo`] walks *up* the directory tree from an
//!   entry's name until it finds one (gopass `LookupTemplate`), and [`render`]
//!   substitutes the predefined gopass template variables.
//!
//! - **Create presets**: a small fixed set of "secret types" (Website login,
//!   PIN code) that build a secret at a specific path from a few field values —
//!   the "create from a few options" flow (gopass `gopass create` wizard).
//!
//! # Rendering subset
//!
//! gopass renders templates with Go `text/template`. This implementation
//! supports the documented predefined variables — `{{ .Content }}`,
//! `{{ .Name }}`, `{{ .Path }}`, `{{ .Dir }}`, `{{ .DirName }}` — which cover
//! the common case (a template that lays out the generated password plus blank
//! fields). Pipe functions (`{{ .Content | md5sum }}`, …) are not yet supported
//! and are reported as an error rather than silently mis-rendered.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::error::{Error, ErrorCode};

/// The gopass content-template filename, placed inside a store directory.
pub const TEMPLATE_FILE: &str = ".pass-template";

/// Variables available to a content template (gopass `payload` struct).
#[derive(Debug, Clone, Copy)]
pub struct TemplateVars<'a> {
    /// The secret payload being created — usually the generated password.
    pub content: &'a str,
    /// Base name of the secret (last path segment).
    pub name: &'a str,
    /// Full entry name (the secret's path).
    pub path: &'a str,
    /// Directory of the secret (parent of the name).
    pub dir: &'a str,
    /// Base name of the secret's directory.
    pub dirname: &'a str,
}

/// Render a `.pass-template` against [`TemplateVars`], substituting
/// `{{ .Content }}`, `{{ .Name }}`, `{{ .Path }}`, `{{ .Dir }}`,
/// `{{ .DirName }}`.
///
/// `{{ ... }}` may be surrounded by arbitrary text; unmatched delimiters are
/// emitted literally. Unknown variables or any expression involving a pipe
/// (`|`) are reported as [`ErrorCode::TemplateError`].
///
/// # Errors
///
/// Returns `TemplateError` for an unknown variable or a pipe expression.
pub fn render(tpl: &str, vars: &TemplateVars<'_>) -> Result<String, Error> {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "{{".len()..];
        if let Some(end) = after.find("}}") {
            let expr = after[..end].trim();
            out.push_str(&eval_var(expr, vars)?);
            rest = &after[end + "}}".len()..];
        } else {
            // No closing }} — emit the rest literally and stop.
            out.push_str(&rest[start..]);
            return Ok(out);
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Evaluate a single `{{ expr }}` body. Only bare `.Variable` is supported.
fn eval_var(expr: &str, vars: &TemplateVars<'_>) -> Result<String, Error> {
    if expr.contains('|') {
        return Err(Error::new(
            ErrorCode::TemplateError,
            "template pipe functions are not supported yet",
        ));
    }
    match expr {
        ".Content" => Ok(vars.content.to_string()),
        ".Name" => Ok(vars.name.to_string()),
        ".Path" => Ok(vars.path.to_string()),
        ".Dir" => Ok(vars.dir.to_string()),
        ".DirName" => Ok(vars.dirname.to_string()),
        _ => Err(Error::new(
            ErrorCode::TemplateError,
            format!("unknown template variable: {expr:?}"),
        )),
    }
}

/// Parent directory of a `/`-separated path, or `""` for a top-level name /
/// root (mirrors Go `filepath.Dir` collapsing to root).
fn dir_of(s: &str) -> &str {
    match s.rfind('/') {
        Some(idx) => &s[..idx],
        None => "",
    }
}

/// Walk *up* the directory tree of `name` and return the content of the first
/// `.pass-template` found in the store, or `None` (gopass `LookupTemplate`).
///
/// Templates are stored as plaintext (like the recipients file), so this reads
/// straight from the worktree. The search starts at the entry's directory and
/// proceeds toward the root, so a nearer template wins.
#[must_use]
pub fn lookup_template_in_repo(repo_path: &Path, name: &str) -> Option<String> {
    let name = name.trim_start_matches('/');
    let mut cur = name;
    loop {
        let prev_len = cur.len();
        cur = dir_of(cur);
        if cur.len() == prev_len {
            break; // no progress — reached the root
        }
        let rel = if cur.is_empty() {
            TEMPLATE_FILE.to_string()
        } else {
            format!("{cur}/{TEMPLATE_FILE}")
        };
        if let Ok(content) = std::fs::read_to_string(repo_path.join(rel)) {
            return Some(content);
        }
    }
    None
}

// ── Create presets (the "create from a few options" flow) ──────────────────

/// One input field of a create preset. Mirrors gopass's create-wizard
/// `Attribute` (`type` / `charset` / `min` / `max` / `strict`) so the UI can
/// render keyboard hints, mask secret fields, and drive the password generator
/// the same way gopass does.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PresetField {
    /// Key stored in the secret body (AKV) and matched against [`CreatePreset::name_from`].
    pub key: &'static str,
    /// Human label shown by the UI.
    pub label: &'static str,
    /// Whether the field must be supplied.
    pub required: bool,
    /// gopass field `type`: `"password"` (generatable + masked), `"hostname"`,
    /// `"string"`, or `"multiline"`. Serialized as `"type"` for gopass compatibility.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// gopass per-attribute `charset`. When set on a `"password"` field, the
    /// generator produces characters only from this set (e.g. `"0123456789"`
    /// for a PIN); `None` means the default alphabet + a mode choice.
    pub charset: Option<&'static str>,
    /// gopass `min` length bound for a generated value.
    pub min: Option<usize>,
    /// gopass `max` length bound for a generated value.
    pub max: Option<usize>,
    /// gopass `strict`: require every character class present in the alphabet
    /// to be represented in a generated value.
    pub strict: bool,
}

/// A built-in secret-creation preset (gopass `gopass create` wizard entry).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CreatePreset {
    /// Stable identifier (e.g. `"website"`).
    pub id: &'static str,
    /// Display label (e.g. `"Website login"`).
    pub label: &'static str,
    /// Directory prefix the secret is generated under (e.g. `"websites"`).
    pub prefix: &'static str,
    /// Field keys whose values are joined to form the secret's name.
    pub name_from: &'static [&'static str],
    /// Fields the UI should collect.
    pub fields: &'static [PresetField],
}

const WEBSITE_PRESET: CreatePreset = CreatePreset {
    id: "website",
    label: "Website login",
    prefix: "websites",
    name_from: &["url", "username"],
    fields: &[
        PresetField {
            key: "url",
            label: "Website URL",
            required: true,
            kind: "hostname",
            charset: None,
            min: None,
            max: None,
            strict: false,
        },
        PresetField {
            key: "username",
            label: "Username",
            required: true,
            kind: "string",
            charset: None,
            min: None,
            max: None,
            strict: false,
        },
        PresetField {
            key: "password",
            label: "Password",
            required: true,
            kind: "password",
            charset: None,
            min: None,
            max: None,
            strict: false,
        },
    ],
};

const PIN_PRESET: CreatePreset = CreatePreset {
    id: "pin",
    label: "PIN Code (numerical)",
    prefix: "pin",
    name_from: &["authority", "application"],
    fields: &[
        PresetField {
            key: "authority",
            label: "Authority",
            required: true,
            kind: "string",
            charset: None,
            min: None,
            max: None,
            strict: false,
        },
        PresetField {
            key: "application",
            label: "Entity",
            required: true,
            kind: "string",
            charset: None,
            min: None,
            max: None,
            strict: false,
        },
        PresetField {
            key: "password",
            label: "PIN",
            required: true,
            kind: "password",
            charset: Some("0123456789"),
            min: Some(1),
            max: Some(64),
            strict: false,
        },
    ],
};

/// The built-in create presets (gopass default wizard templates).
#[must_use]
pub fn builtin_presets() -> &'static [CreatePreset] {
    &[WEBSITE_PRESET, PIN_PRESET]
}

/// Look up a preset by id.
#[must_use]
pub fn find_preset(id: &str) -> Option<&'static CreatePreset> {
    builtin_presets().iter().find(|p| p.id == id)
}

/// Sanitize one path segment of a generated name (gopass `CleanFilename`):
/// keep alphanumerics and a few safe punctuation chars; replace the rest with
/// `-` so a URL/username can't escape the prefix or inject path separators.
fn sanitize_name_part(s: &str) -> String {
    let cleaned: String = s
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | '@') {
                c
            } else {
                '-'
            }
        })
        .collect();
    cleaned.trim_matches('-').to_string()
}

/// Build the secret name for a preset from the supplied field values:
/// `<prefix>/<part0>/<part1>/…` where parts come from [`CreatePreset::name_from`].
///
/// # Errors
///
/// Returns `InvalidEntryName` if a required `name_from` field is missing or
/// empty (so the generated name would be degenerate).
pub fn preset_name<S: ::std::hash::BuildHasher>(
    preset: &CreatePreset,
    fields: &HashMap<&str, String, S>,
) -> Result<String, Error> {
    let mut parts: Vec<String> = Vec::new();
    for key in preset.name_from {
        let raw = fields.get(key).map_or("", String::as_str);
        let part = sanitize_name_part(raw);
        if part.is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidEntryName,
                format!("preset field {key:?} is required to build the name"),
            ));
        }
        parts.push(part);
    }
    Ok(format!("{}/{}", preset.prefix, parts.join("/")))
}

/// Build the gopass AKV secret body for a preset: the `password` field value
/// on the first line, then `key: value` lines for every other supplied field.
///
/// # Errors
///
/// Returns `InvalidEntryName` if the required `password` field is missing.
pub fn preset_body<S: ::std::hash::BuildHasher>(
    preset: &CreatePreset,
    fields: &HashMap<&str, String, S>,
) -> Result<Vec<u8>, Error> {
    let password = fields.get("password").map(String::as_str).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidEntryName,
            "preset requires a \"password\" field",
        )
    })?;

    let mut body = String::new();
    body.push_str(password);
    for field in preset.fields {
        if field.key == "password" {
            continue;
        }
        if let Some(value) = fields.get(field.key) {
            body.push('\n');
            body.push_str(field.key);
            body.push_str(": ");
            body.push_str(value);
        }
    }
    Ok(body.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_content_and_name() {
        let vars = TemplateVars {
            content: "s3kr3t",
            name: "gmail",
            path: "email/gmail",
            dir: "email",
            dirname: "email",
        };
        let tpl = "{{ .Content }}\n\nuser: \nname: {{ .Name }}\n";
        let out = render(tpl, &vars).unwrap();
        assert_eq!(out, "s3kr3t\n\nuser: \nname: gmail\n");
    }

    #[test]
    fn render_passes_through_literal_text() {
        let vars = TemplateVars {
            content: "x",
            name: "x",
            path: "x",
            dir: "",
            dirname: "",
        };
        assert_eq!(
            render("no templates here", &vars).unwrap(),
            "no templates here"
        );
        assert_eq!(render("", &vars).unwrap(), "");
    }

    #[test]
    fn render_unknown_variable_errors() {
        let vars = TemplateVars {
            content: "x",
            name: "x",
            path: "x",
            dir: "",
            dirname: "",
        };
        let err = render("{{ .Nope }}", &vars).unwrap_err();
        assert_eq!(err.code, "TEMPLATE_ERROR");
    }

    #[test]
    fn render_pipe_function_errors() {
        let vars = TemplateVars {
            content: "x",
            name: "x",
            path: "x",
            dir: "",
            dirname: "",
        };
        let err = render("{{ .Content | md5sum }}", &vars).unwrap_err();
        assert_eq!(err.code, "TEMPLATE_ERROR");
        assert!(err.message.contains("pipe"));
    }

    #[test]
    fn render_unmatched_delimiter_emitted_literally() {
        let vars = TemplateVars {
            content: "x",
            name: "x",
            path: "x",
            dir: "",
            dirname: "",
        };
        assert_eq!(render("a {{ .Content b", &vars).unwrap(), "a {{ .Content b");
    }

    #[test]
    fn render_handles_unicode() {
        let vars = TemplateVars {
            content: "密码",
            name: "x",
            path: "x",
            dir: "",
            dirname: "",
        };
        assert_eq!(render("pw={{ .Content }}", &vars).unwrap(), "pw=密码");
    }

    #[test]
    fn dir_of_walks_up() {
        assert_eq!(dir_of("a/b/c"), "a/b");
        assert_eq!(dir_of("a"), "");
        assert_eq!(dir_of(""), "");
    }

    #[test]
    fn lookup_walks_up_to_find_template() {
        let dir = tempfile::tempdir().unwrap();
        // websites/.pass-template applies to websites/foo and websites/a/b.
        std::fs::create_dir_all(dir.path().join("websites/sub")).unwrap();
        std::fs::write(
            dir.path().join("websites").join(TEMPLATE_FILE),
            "{{ .Content }}\n\nuser: ",
        )
        .unwrap();

        let tpl = lookup_template_in_repo(dir.path(), "websites/foo").unwrap();
        assert!(tpl.contains("user:"));
        // Nearer template wins over a root one.
        std::fs::write(dir.path().join(TEMPLATE_FILE), "ROOT").unwrap();
        let tpl = lookup_template_in_repo(dir.path(), "websites/foo").unwrap();
        assert!(tpl.contains("user:"), "nearer websites/.pass-template wins");
        // Deeper entry still finds the websites template.
        let tpl = lookup_template_in_repo(dir.path(), "websites/sub/deep").unwrap();
        assert!(tpl.contains("user:"));
        // Unrelated entry falls back to the root template.
        let tpl = lookup_template_in_repo(dir.path(), "misc/x").unwrap();
        assert_eq!(tpl, "ROOT");
        // No template at all.
        let none = lookup_template_in_repo(dir.path(), "empty/x");
        assert!(none.is_none() || none.as_deref() == Some("ROOT"));
    }

    #[test]
    fn sanitize_keeps_safe_chars() {
        assert_eq!(sanitize_name_part("example.com"), "example.com");
        assert_eq!(sanitize_name_part("a/b"), "a-b");
        assert_eq!(sanitize_name_part("  hi there!  "), "hi-there");
        assert_eq!(sanitize_name_part("user@host"), "user@host");
    }

    fn fields(pairs: &[(&'static str, &str)]) -> HashMap<&'static str, String> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    #[test]
    fn preset_name_website() {
        let preset = find_preset("website").unwrap();
        let f = fields(&[("url", "example.com"), ("username", "alice")]);
        let name = preset_name(preset, &f).unwrap();
        assert_eq!(name, "websites/example.com/alice");
    }

    #[test]
    fn preset_name_sanitizes_url_scheme_and_slashes() {
        // A full URL: ':' and both '/' become '-', so it can't inject segments.
        let preset = find_preset("website").unwrap();
        let f = fields(&[("url", "https://example.com"), ("username", "alice")]);
        let name = preset_name(preset, &f).unwrap();
        assert_eq!(name, "websites/https---example.com/alice");
    }

    #[test]
    fn preset_name_missing_required_part_errors() {
        let preset = find_preset("website").unwrap();
        let f = fields(&[("url", ""), ("username", "alice")]);
        let err = preset_name(preset, &f).unwrap_err();
        assert_eq!(err.code, "INVALID_ENTRY_NAME");
    }

    #[test]
    fn preset_body_akv_format() {
        let preset = find_preset("pin").unwrap();
        let f = fields(&[
            ("authority", "bank"),
            ("application", "app"),
            ("password", "1234"),
        ]);
        let body = preset_body(preset, &f).unwrap();
        let text = String::from_utf8(body).unwrap();
        assert_eq!(text, "1234\nauthority: bank\napplication: app");
        // Password is always the first line.
        assert_eq!(text.lines().next().unwrap(), "1234");
    }

    #[test]
    fn preset_body_requires_password() {
        let preset = find_preset("website").unwrap();
        let f = fields(&[("url", "x"), ("username", "y")]);
        let err = preset_body(preset, &f).unwrap_err();
        assert_eq!(err.code, "INVALID_ENTRY_NAME");
    }

    #[test]
    fn builtin_presets_have_unique_ids() {
        let presets = builtin_presets();
        let ids: Vec<_> = presets.iter().map(|p| p.id).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "preset ids must be unique");
        assert!(
            presets
                .iter()
                .all(|p| p.fields.iter().any(|f| f.key == "password"))
        );
    }
}
