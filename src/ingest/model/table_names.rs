//! Rails' `compute_table_name`, resolved across files.
//!
//! A class nested under a concrete (non-abstract) ActiveRecord model is
//! stored in the parent's singular table name plus its own plural:
//! `Community::Advocate` is `community_advocates` when `Community` is a
//! model, but `advocates` when `Community` is a plain module. Whether
//! the parent is a model is a fact about ANOTHER file, so it is gathered
//! in the pre-pass ([`ModelDecls`]) and handed to model ingest as
//! [`TableNames::nested`].

use std::collections::HashMap;

use super::super::library_class::ModelBases;
use super::super::util::{class_name_path, constant_path_of, find_all_classes_with_scope};
use super::{TablePrefixes, explicit_table_prefix, parse_table_name_decl};

/// Table-name inputs that depend on other files, built once after the
/// pre-pass and read by every model's ingest.
#[derive(Debug, Default)]
pub struct TableNames {
    /// Each abstract base's inherited `table_name_prefix`
    /// ([`super::inherited_table_prefixes`]).
    pub base_prefixes: TablePrefixes,
    /// Full table name of every class nested under a concrete model.
    pub nested: HashMap<String, String>,
}

/// Rails' convention for a class that is neither nested under a model
/// nor given a `self.table_name`: the namespace's prefix (the class's
/// own wins, else the nearest module that declares one, else the one an
/// abstract base set) followed by the demodulized, pluralized name.
pub(super) fn conventional_table_name(
    class_name: &str,
    own_prefix: Option<String>,
    prefixes: &TablePrefixes,
    inherited_prefix: Option<&String>,
) -> String {
    let mut segments: Vec<&str> = class_name.split("::").collect();
    segments.pop();
    let mut prefix = own_prefix.unwrap_or_default();
    let mut from_module = false;
    while prefix.is_empty() && !segments.is_empty() {
        if let Some(p) = prefixes.get(&segments.join("::")) {
            prefix = p.clone();
            from_module = true;
            break;
        }
        segments.pop();
    }
    // No module parent declares one (an empty one still counts): the
    // class attribute its abstract base set.
    if prefix.is_empty()
        && !from_module
        && let Some(inherited) = inherited_prefix
    {
        prefix = inherited.clone();
    }
    format!("{prefix}{}", crate::naming::rails_table_name(class_name))
}

/// What the pre-pass reads off one class definition.
#[derive(Debug)]
struct Decl {
    scope: Vec<String>,
    superclass: Vec<String>,
    table_name: Option<String>,
    own_prefix: Option<String>,
}

/// Every `class X < Y` the models directories and support roots declare,
/// by full name, with what its table name depends on.
#[derive(Debug, Default)]
pub struct ModelDecls {
    decls: HashMap<String, Decl>,
}

impl ModelDecls {
    pub fn record(&mut self, source: &[u8], file: &str) {
        let result = super::super::prism::parse(source, file);
        let root = result.node();
        for (scope, class) in find_all_classes_with_scope(&root) {
            let Some(path) = class_name_path(&class) else {
                continue;
            };
            let Some(superclass) = class.superclass().and_then(|n| constant_path_of(&n)) else {
                continue;
            };
            let table_name = class
                .body()
                .and_then(|body| parse_table_name_decl(body, file).ok().flatten())
                .map(|(name, _)| name);
            let own_prefix = class.body().and_then(explicit_table_prefix);
            let mut full = scope.clone();
            full.extend(path);
            self.decls.insert(
                full.join("::"),
                Decl {
                    scope,
                    superclass,
                    table_name,
                    own_prefix,
                },
            );
        }
    }

    /// Resolve the nested names, once `bases` has been closed over.
    pub fn resolve(
        &self,
        bases: &ModelBases,
        prefixes: &TablePrefixes,
        base_prefixes: TablePrefixes,
    ) -> TableNames {
        let resolver = Resolver {
            decls: &self.decls,
            bases,
            prefixes,
            base_prefixes: &base_prefixes,
        };
        let nested = self
            .decls
            .iter()
            .filter(|(_, decl)| bases.contains(&resolver.parent_of(decl)))
            .filter_map(|(name, _)| Some((name.clone(), resolver.nested_table(name, 0)?)))
            .collect();
        TableNames {
            base_prefixes,
            nested,
        }
    }
}

/// Guards the walks below against a cyclic hierarchy.
const MAX_DEPTH: usize = 16;

struct Resolver<'a> {
    decls: &'a HashMap<String, Decl>,
    bases: &'a ModelBases,
    prefixes: &'a TablePrefixes,
    base_prefixes: &'a TablePrefixes,
}

impl Resolver<'_> {
    /// The class's superclass as Ruby would find it: an abstract base by
    /// the way model ingest resolves it, else a model declared in the
    /// scope or any enclosing one.
    fn parent_of(&self, decl: &Decl) -> String {
        let resolved = self.bases.resolve_superclass(&decl.scope, &decl.superclass);
        if self.bases.contains(&resolved) {
            return resolved;
        }
        let tail = decl.superclass.join("::");
        (0..=decl.scope.len())
            .rev()
            .map(|depth| {
                decl.scope[..depth]
                    .iter()
                    .cloned()
                    .chain([tail.clone()])
                    .collect::<Vec<_>>()
                    .join("::")
            })
            .find(|candidate| self.decls.contains_key(candidate))
            .unwrap_or(resolved)
    }

    /// The table of a concrete model, `None` for anything else (an
    /// abstract base, a plain class, a module).
    fn table_of(&self, name: &str, depth: usize) -> Option<String> {
        if depth > MAX_DEPTH || self.bases.contains(name) {
            return None;
        }
        let decl = self.decls.get(name)?;
        let parent = self.parent_of(decl);
        if self.bases.contains(&parent) {
            return Some(
                match (&decl.table_name, self.nested_table(name, depth + 1)) {
                    (Some(explicit), _) => explicit.clone(),
                    (None, Some(nested)) => nested,
                    (None, None) => {
                        let inherited = self.base_prefixes.get(&parent);
                        conventional_table_name(
                            name,
                            decl.own_prefix.clone(),
                            self.prefixes,
                            inherited,
                        )
                    }
                },
            );
        }
        // Single-table inheritance: the base's table unless redeclared.
        let base_table = self.table_of(&parent, depth + 1)?;
        Some(decl.table_name.clone().unwrap_or(base_table))
    }

    /// The table of a class that sits directly under a concrete model:
    /// the parent's singular table name, `_`, then its own plural. Rails
    /// takes the prefix from that parent too, since a model class is the
    /// first module parent to answer `table_name_prefix`.
    fn nested_table(&self, name: &str, depth: usize) -> Option<String> {
        let (enclosing, _) = name.rsplit_once("::")?;
        let parent_table = self.table_of(enclosing, depth + 1)?;
        let prefix = self.class_prefix(enclosing, depth + 1);
        Some(format!(
            "{prefix}{}_{}",
            crate::naming::singularize(&parent_table),
            crate::naming::rails_table_name(name)
        ))
    }

    /// `table_name_prefix` as a class attribute: the class's own, else
    /// the nearest ancestor's.
    fn class_prefix(&self, name: &str, depth: usize) -> String {
        let Some(decl) = self.decls.get(name).filter(|_| depth <= MAX_DEPTH) else {
            return String::new();
        };
        if let Some(own) = &decl.own_prefix {
            return own.clone();
        }
        let parent = self.parent_of(decl);
        match self.base_prefixes.get(&parent) {
            Some(inherited) => inherited.clone(),
            None => self.class_prefix(&parent, depth + 1),
        }
    }
}
