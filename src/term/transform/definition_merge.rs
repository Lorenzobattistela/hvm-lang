use super::simplify_ref_to_ref::subst_ref_to_ref;
use crate::term::{Book, Definition, Name, Rule, Term};
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use std::collections::BTreeMap;

impl Book {
  /// Merges definitions that have the same structure into one definition.
  /// Expects variables to be linear.
  ///
  /// Ignores origin of the rules when merging,
  /// Should not be preceded by passes that cares about the origins.
  pub fn merge_definitions(&mut self, main: &Name) {
    let defs: Vec<_> = self.defs.keys().cloned().collect();
    self.merge(main, defs.into_iter());
  }

  /// Checks and merges identical definitions given by `defs`.
  /// We never merge the entrypoint function with something else.
  fn merge(&mut self, main: &Name, defs: impl Iterator<Item = Name>) {
    // Sets of definitions that are identical, indexed by the body term.
    let equal_terms = self.collect_terms(defs.filter(|def_name| def_name != main));

    // Map of old name to new merged name
    let mut name_map = BTreeMap::new();

    for (term, equal_defs) in equal_terms {
      // def1_$_def2_$_def3
      let new_name = Name::from(equal_defs.iter().join("_$_"));

      // Builtin origin takes precedence
      let builtin = equal_defs.iter().any(|nam| self.defs[nam].builtin);

      if equal_defs.len() > 1 {
        // Merging some defs
        // Add the merged def
        let new_def =
          Definition { name: new_name.clone(), rules: vec![Rule { pats: vec![], body: term }], builtin };
        self.defs.insert(new_name.clone(), new_def);
        // Remove the old ones and write the map of old names to new ones.
        for name in equal_defs {
          self.defs.swap_remove(&name);
          name_map.insert(name, new_name.clone());
        }
      } else {
        // Not merging, just put the body back
        let def_name = equal_defs.into_iter().next().unwrap();
        let def = self.defs.get_mut(&def_name).unwrap();
        def.rule_mut().body = term;
      }
    }
    self.update_refs(&name_map, main);
  }

  fn collect_terms(&mut self, def_entries: impl Iterator<Item = Name>) -> IndexMap<Term, IndexSet<Name>> {
    let mut equal_terms: IndexMap<Term, IndexSet<Name>> = IndexMap::new();

    for def_name in def_entries {
      let def = self.defs.get_mut(&def_name).unwrap();
      let term = std::mem::take(&mut def.rule_mut().body);
      equal_terms.entry(term).or_default().insert(def_name);
    }

    equal_terms
  }

  fn update_refs(&mut self, name_map: &BTreeMap<Name, Name>, main: &Name) {
    let mut updated_defs = Vec::new();

    for def in self.defs.values_mut() {
      if subst_ref_to_ref(&mut def.rule_mut().body, name_map) {
        updated_defs.push(def.name.clone());
      }
    }

    if !updated_defs.is_empty() {
      self.merge(main, updated_defs.into_iter());
    }
  }
}
