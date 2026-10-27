//! Project the existing serde contract into foreign records without duplicating schemas.
//! Core remains independent of UniFFI. Only ABI representation changes live here.
use quote::{quote, ToTokens};
use std::{env, fs, path::PathBuf};
use syn::{visit_mut::VisitMut, Attribute, GenericArgument, Item, PathArguments, Type};

fn attrs(attrs: &mut Vec<Attribute>) {
    attrs.retain(|a| {
        a.path().is_ident("serde")
            || a.path().is_ident("doc")
            || a.path().is_ident("default")
            || a.path().is_ident("allow")
    });
    // Defaults affect deserializing the transport, not domain validation. Generated
    // records receive complete fields from UniFFI, so callers must supply domain
    // defaults explicitly; this does not preserve Core's custom serde defaults.
    for attr in attrs.iter_mut() {
        if attr.path().is_ident("serde") {
            let text = attr.to_token_stream().to_string();
            if text.contains("default_") {
                *attr = syn::parse_quote!(#[serde(default)]);
            }
        }
    }
}
/// `#[serde(skip)]`, which leaves a field out of the serde text both ways.
fn serde_skip(attr: &Attribute) -> bool {
    let mut skip = false;
    if attr.path().is_ident("serde") {
        let _ = attr.parse_nested_meta(|meta| {
            skip |= meta.path.is_ident("skip");
            Ok(())
        });
    }
    skip
}

struct ForeignTypes;
impl VisitMut for ForeignTypes {
    fn visit_attribute_mut(&mut self, _: &mut Attribute) {}
    fn visit_type_mut(&mut self, ty: &mut Type) {
        match ty {
            Type::Array(array) => {
                let mut item = *array.elem.clone();
                self.visit_type_mut(&mut item);
                *ty = syn::parse_quote!(Vec<#item>);
            }
            Type::Reference(_) => *ty = syn::parse_quote!(String),
            Type::Tuple(_) => *ty = syn::parse_quote!(JsonValue),
            Type::Path(path) if path.path.is_ident("Value") => *ty = syn::parse_quote!(JsonValue),
            Type::Path(path)
                if path.path.segments.len() == 2
                    && path.path.segments[0].ident == "serde_json"
                    && path.path.segments[1].ident == "Value" =>
            {
                *ty = syn::parse_quote!(JsonValue)
            }
            // Timestamps cross as their RFC 3339 serde text; UniFFI has no chrono type.
            Type::Path(path)
                if path
                    .path
                    .segments
                    .last()
                    .is_some_and(|part| part.ident == "DateTime") =>
            {
                *ty = syn::parse_quote!(String)
            }
            // Persisted document inputs keep their existing SDK JSON representation.
            // Core validates them against its typed serde contracts on entry.
            Type::Path(path)
                if path
                    .path
                    .segments
                    .first()
                    .is_some_and(|part| part.ident == "crate")
                    && path
                        .path
                        .segments
                        .iter()
                        .any(|part| part.ident == "contracts") =>
            {
                *ty = syn::parse_quote!(JsonValue);
            }
            // Core shares large values behind `Arc`; serde writes the value itself.
            Type::Path(path)
                if path
                    .path
                    .segments
                    .last()
                    .is_some_and(|part| part.ident == "Arc") =>
            {
                if let Some(PathArguments::AngleBracketed(arguments)) =
                    path.path.segments.last().map(|part| &part.arguments)
                {
                    if let Some(GenericArgument::Type(inner)) = arguments.args.first() {
                        *ty = inner.clone();
                        self.visit_type_mut(ty);
                    }
                }
            }
            Type::Path(path) if path.path.is_ident("usize") => *ty = syn::parse_quote!(u64),
            Type::Path(path) if path.path.is_ident("Task") => *ty = syn::parse_quote!(CloudTask),
            Type::Path(path) => {
                for segment in &mut path.path.segments {
                    if segment.ident == "BTreeMap" {
                        segment.ident = syn::parse_quote!(HashMap);
                    }
                }
                syn::visit_mut::visit_type_mut(self, ty);
            }
            _ => syn::visit_mut::visit_type_mut(self, ty),
        }
    }
}
/// Sync names that would shadow platform types in the foreign shells
/// (SwiftUI `View`/`Label`, Apple's `Network` module, `android.net.Network`).
/// Only the Rust identifier changes; serde field and variant names, and so the
/// JSON projection, stay the same.
const SYNC_RENAMES: [(&str, &str); 5] = [
    ("View", "SyncView"),
    ("Label", "SyncLabel"),
    ("Network", "SyncNetwork"),
    ("Scope", "SyncScope"),
    ("Priority", "SyncPriority"),
];
fn sync_name(ident: &syn::Ident) -> Option<syn::Ident> {
    SYNC_RENAMES
        .iter()
        .find(|(from, _)| ident == from)
        .map(|(_, to)| syn::Ident::new(to, ident.span()))
}
struct SyncRenames;
impl VisitMut for SyncRenames {
    fn visit_attribute_mut(&mut self, _: &mut Attribute) {}
    fn visit_type_path_mut(&mut self, path: &mut syn::TypePath) {
        for segment in &mut path.path.segments {
            if let Some(name) = sync_name(&segment.ident) {
                segment.ident = name;
            }
        }
        syn::visit_mut::visit_type_path_mut(self, path);
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let core = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../app-core/src");
    let files = [
        "workspace.rs",
        "workspace_changes.rs",
        "study_profile.rs",
        "schedule/mod.rs",
        "schedule/timing.rs",
        "schedule/requests.rs",
        "schedule/course_detail.rs",
        "schedule/course_content.rs",
        "schedule/exams.rs",
        "schedule/search.rs",
        "schedule/calendar.rs",
        "schedule/occurrence.rs",
        "schedule/alternation.rs",
        "schedule/tasks.rs",
        "schedule/today.rs",
        "schedule/reminders.rs",
        "schedule/ics.rs",
        "course_query/mod.rs",
        "catalog.rs",
        "course_query/contracts.rs",
        "school_auth.rs",
        "school_service.rs",
        "school_browser/mod.rs",
        "school_browser/port.rs",
        "lifecycle.rs",
        "cloud_sync/mod.rs",
        "sync_engine/mod.rs",
        "course_materials.rs",
        "storage.rs",
        "notes.rs",
        "../../adpt-isct/src/lms.rs",
        "../../adpt-isct/src/kyomu/search.rs",
        "../../app-ocr/src/recognition.rs",
    ];
    let mut output = quote! { use crate::json::JsonValue; use serde::{Serialize, Deserialize}; use std::collections::HashMap; };
    for name in files {
        let path = core.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let file = syn::parse_file(&fs::read_to_string(path)?)?;
        for item in file.items {
            match item {
                Item::Struct(mut item)
                    if matches!(item.fields, syn::Fields::Named(_))
                        && matches!(item.vis, syn::Visibility::Public(_))
                        && item.attrs.iter().any(|a| {
                            a.path().is_ident("derive")
                                && a.to_token_stream().to_string().contains("Serialize")
                                || a.path().is_ident("derive")
                                    && a.to_token_stream().to_string().contains("Deserialize")
                        }) =>
                {
                    if name == "course_query/contracts.rs"
                        && item.ident != "FinderRowView"
                        && item.ident != "OfficialSelection"
                    {
                        continue;
                    }
                    if name == "../../adpt-isct/src/lms.rs" && item.ident != "ResolvedFile" {
                        continue;
                    }
                    if name == "../../adpt-isct/src/kyomu/search.rs"
                        && item.ident != "OfficialFilterOption"
                    {
                        continue;
                    }
                    // Service handles and private implementation state never enter metadata.
                    let default = item.attrs.iter().any(|a| {
                        a.path().is_ident("derive")
                            && a.to_token_stream().to_string().contains("Default")
                    });
                    attrs(&mut item.attrs);
                    item.attrs.insert(0, syn::parse_quote!(#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]));
                    if default {
                        item.attrs.insert(0, syn::parse_quote!(#[derive(Default)]));
                    }
                    if item.ident == "Task" {
                        item.ident = syn::parse_quote!(CloudTask);
                    }
                    if name == "sync_engine/mod.rs" {
                        if let Some(renamed) = sync_name(&item.ident) {
                            item.ident = renamed;
                        }
                        SyncRenames.visit_item_struct_mut(&mut item);
                    }
                    // A field serde skips stays in Core: records cross as serde text.
                    if let syn::Fields::Named(fields) = &mut item.fields {
                        fields.named = std::mem::take(&mut fields.named)
                            .into_iter()
                            .filter(|field| !field.attrs.iter().any(serde_skip))
                            .collect();
                    }
                    for field in &mut item.fields {
                        attrs(&mut field.attrs);
                        ForeignTypes.visit_type_mut(&mut field.ty);
                    }
                    output.extend(quote!(#item));
                }
                Item::Enum(mut item)
                    if matches!(item.vis, syn::Visibility::Public(_))
                        && item.attrs.iter().any(|a| {
                            a.path().is_ident("derive")
                                && a.to_token_stream().to_string().contains("Serialize")
                                || a.path().is_ident("derive")
                                    && a.to_token_stream().to_string().contains("Deserialize")
                        }) =>
                {
                    let default = item.attrs.iter().any(|a| {
                        a.path().is_ident("derive")
                            && a.to_token_stream().to_string().contains("Default")
                    });
                    if name == "course_query/contracts.rs" && item.ident != "Suggestion" {
                        continue;
                    }
                    if name.starts_with("../../adpt-isct/") {
                        continue;
                    }
                    if name == "sync_engine/mod.rs" {
                        if let Some(renamed) = sync_name(&item.ident) {
                            item.ident = renamed;
                        }
                        SyncRenames.visit_item_enum_mut(&mut item);
                    }
                    attrs(&mut item.attrs);
                    item.attrs.insert(0, syn::parse_quote!(#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Enum)]));
                    if default {
                        item.attrs.insert(0, syn::parse_quote!(#[derive(Default)]));
                    }
                    for variant in &mut item.variants {
                        attrs(&mut variant.attrs);
                        for field in &mut variant.fields {
                            attrs(&mut field.attrs);
                            ForeignTypes.visit_type_mut(&mut field.ty);
                        }
                    }
                    output.extend(quote!(#item));
                }
                _ => {}
            }
        }
    }
    output.extend(quote!(
        fn false_value(value: &bool) -> bool {
            !*value
        }
    ));
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("contracts.rs"),
        output.to_string(),
    )?;
    Ok(())
}
