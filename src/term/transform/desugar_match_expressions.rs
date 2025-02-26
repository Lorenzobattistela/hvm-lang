use crate::{
  term::{display::DisplayJoin, Book, Definition, MatchNum, Name, Op, Pattern, Rule, Tag, Term, Type},
  Warning,
};
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use std::collections::{BTreeSet, HashSet};

impl Book {
  /// Extracts adt match terms into pattern matching functions.
  /// Creates rules with potentially nested patterns, so the flattening pass needs to be called after.
  pub fn extract_adt_matches(&mut self, warnings: &mut Vec<Warning>) -> Result<(), String> {
    let mut new_defs = vec![];
    for (def_name, def) in &mut self.defs {
      for rule in def.rules.iter_mut() {
        rule
          .body
          .extract_adt_matches(def_name, def.builtin, &self.ctrs, &mut new_defs, &mut 0, warnings)
          .map_err(|e| format!("In definition '{def_name}': {e}"))?;
      }
    }
    self.defs.extend(new_defs);
    Ok(())
  }

  /// Converts tuple and var matches into let expressions,
  /// makes num matches have exactly one rule for zero and one rule for succ.
  /// Should be run after pattern matching functions are desugared.
  pub fn normalize_native_matches(&mut self) -> Result<(), String> {
    for (def_name, def) in self.defs.iter_mut() {
      def
        .rule_mut()
        .body
        .normalize_native_matches(&self.ctrs)
        .map_err(|e| format!("In definition '{def_name}': {e}"))?;
    }
    Ok(())
  }
}

#[derive(Debug)]
pub enum MatchError {
  Empty,
  Infer(String),
  Repeated(Name),
  Missing(HashSet<Name>),
  LetPat(Box<MatchError>),
  Linearize(Name),
}

impl std::error::Error for MatchError {}

impl std::fmt::Display for MatchError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    fn ctrs_plural_or_sing(n: usize) -> &'static str {
      if n > 1 { "constructors" } else { "a constructor" }
    }

    match self {
      MatchError::Empty => write!(f, "Empty match block found"),
      MatchError::Infer(err) => write!(f, "{err}"),
      MatchError::Repeated(bind) => write!(f, "Repeated var name in a match block: {}", bind),
      MatchError::Missing(names) => {
        let constructor = ctrs_plural_or_sing(names.len());
        let missing = DisplayJoin(|| names.iter(), ", ");
        write!(f, "Missing {constructor} in a match block: {missing}")
      }
      MatchError::LetPat(err) => {
        let let_err = err.to_string().replace("match block", "let bind");
        write!(f, "{let_err}")?;

        if matches!(err.as_ref(), MatchError::Missing(_)) {
          write!(f, "\nConsider using a match block instead")?;
        }

        Ok(())
      }
      MatchError::Linearize(var) => write!(f, "Unable to linearize variable {var} in a match block."),
    }
  }
}

//== ADT match to pattern matching function ==//

impl Term {
  fn extract_adt_matches(
    &mut self,
    def_name: &Name,
    builtin: bool,
    ctrs: &IndexMap<Name, Name>,
    new_defs: &mut Vec<(Name, Definition)>,
    match_count: &mut usize,
    warnings: &mut Vec<Warning>,
  ) -> Result<(), MatchError> {
    match self {
      Term::Mat { matched: box Term::Var { .. }, arms } => {
        let all_vars = arms.iter().all(|(pat, ..)| matches!(pat, Pattern::Var(..)));
        if all_vars && arms.len() > 1 {
          warnings.push(crate::Warning::MatchOnlyVars { def_name: def_name.clone() });
        }
        for (_, term) in arms.iter_mut() {
          term.extract_adt_matches(def_name, builtin, ctrs, new_defs, match_count, warnings)?;
        }
        let matched_type = infer_match_type(arms.iter().map(|(x, _)| x), ctrs)?;
        match matched_type {
          // Don't extract non-adt matches.
          Type::None | Type::Any | Type::Num => (),
          // TODO: Instead of extracting tuple matches, we should flatten one layer and check sub-patterns for something to extract.
          // For now, to prevent extraction we can use `let (a, b) = ...;`
          Type::Adt(_) | Type::Tup => {
            *match_count += 1;
            let match_term = linearize_match_unscoped_vars(self)?;
            let match_term = linearize_match_free_vars(match_term);
            let Term::Mat { matched: box Term::Var { nam }, arms } = match_term else { unreachable!() };
            *match_term = match_to_def(nam, arms, def_name, builtin, new_defs, *match_count);
          }
        }
      }

      Term::Lam { bod, .. } | Term::Chn { bod, .. } => {
        bod.extract_adt_matches(def_name, builtin, ctrs, new_defs, match_count, warnings)?;
      }
      Term::App { fun: fst, arg: snd, .. }
      | Term::Let { pat: Pattern::Var(_), val: fst, nxt: snd }
      | Term::Dup { val: fst, nxt: snd, .. }
      | Term::Tup { fst, snd }
      | Term::Sup { fst, snd, .. }
      | Term::Opx { fst, snd, .. } => {
        fst.extract_adt_matches(def_name, builtin, ctrs, new_defs, match_count, warnings)?;
        snd.extract_adt_matches(def_name, builtin, ctrs, new_defs, match_count, warnings)?;
      }
      Term::Var { .. }
      | Term::Lnk { .. }
      | Term::Num { .. }
      | Term::Str { .. }
      | Term::Ref { .. }
      | Term::Era
      | Term::Err => {}

      Term::Lst { .. } => unreachable!(),
      Term::Mat { .. } => unreachable!("Scrutinee of match expression should have been extracted already"),
      Term::Let { pat, .. } => {
        unreachable!("Destructor let expression should have been desugared already. {pat}")
      }
    }

    Ok(())
  }
}

/// Transforms a match into a new definition with every arm of `arms` as a rule.
/// The result is the new def applied to the scrutinee followed by the free vars of the arms.
fn match_to_def(
  matched_var: &Name,
  arms: &[(Pattern, Term)],
  def_name: &Name,
  builtin: bool,
  new_defs: &mut Vec<(Name, Definition)>,
  match_count: usize,
) -> Term {
  let rules = arms.iter().map(|(pat, term)| Rule { pats: vec![pat.clone()], body: term.clone() }).collect();
  let new_name = Name::from(format!("{def_name}$match${match_count}"));
  let def = Definition { name: new_name.clone(), rules, builtin };
  new_defs.push((new_name.clone(), def));

  Term::arg_call(Term::Ref { nam: new_name }, matched_var.clone())
}

//== Native match normalization ==//

impl Term {
  fn normalize_native_matches(&mut self, ctrs: &IndexMap<Name, Name>) -> Result<(), MatchError> {
    match self {
      Term::Mat { matched: box Term::Var { nam }, arms } => {
        for (_, body) in arms.iter_mut() {
          body.normalize_native_matches(ctrs)?;
        }

        let matched_type = infer_match_type(arms.iter().map(|(x, _)| x), ctrs)?;
        match matched_type {
          Type::None => {
            return Err(MatchError::Empty);
          }
          // This match is useless, will always go with the first rule.
          // TODO: Throw a warning in this case
          Type::Any => {
            // Inside the match we renamed the variable, so we need to restore the name before the match to remove it.
            let fst_arm = &mut arms[0];
            let Pattern::Var(var) = &fst_arm.0 else { unreachable!() };
            let body = &mut fst_arm.1;
            if let Some(var) = var {
              body.subst(var, &Term::Var { nam: nam.clone() });
            }
            *self = std::mem::take(body);
          }
          // TODO: Don't extract tup matches earlier, only flatten earlier and then deal with them here.
          Type::Tup => unreachable!(),
          // Matching on nums is a primitive operation, we can leave it as is.
          // Not extracting into a separate definition allows us to create very specific inets with the MATCH node.
          Type::Num => {
            let match_term = linearize_match_free_vars(self);
            normalize_num_match(match_term)?;
          }
          Type::Adt(_) => unreachable!("Adt match expressions should have been removed earlier"),
        }
      }
      Term::Mat { .. } => unreachable!("Scrutinee of match expression should have been extracted already"),
      Term::Let { val: fst, nxt: snd, .. }
      | Term::App { fun: fst, arg: snd, .. }
      | Term::Tup { fst, snd }
      | Term::Dup { val: fst, nxt: snd, .. }
      | Term::Sup { fst, snd, .. }
      | Term::Opx { fst, snd, .. } => {
        fst.normalize_native_matches(ctrs)?;
        snd.normalize_native_matches(ctrs)?;
      }
      Term::Lam { bod, .. } | Term::Chn { bod, .. } => {
        bod.normalize_native_matches(ctrs)?;
      }
      Term::Var { .. }
      | Term::Lnk { .. }
      | Term::Num { .. }
      | Term::Str { .. }
      | Term::Ref { .. }
      | Term::Era
      | Term::Err => (),
      Term::Lst { .. } => unreachable!(),
    }
    Ok(())
  }
}

/// Transforms a match on Num with any possible patterns into 'match x {0: ...; +: @x-1 ...}'.
fn normalize_num_match(term: &mut Term) -> Result<(), MatchError> {
  let Term::Mat { matched: _, arms } = term else { unreachable!() };

  let mut zero_arm = None;
  for (pat, body) in arms.iter_mut() {
    match pat {
      Pattern::Num(MatchNum::Zero) => {
        zero_arm = Some((Pattern::Num(MatchNum::Zero), std::mem::take(body)));
        break;
      }
      Pattern::Var(var) => {
        if let Some(var) = var {
          body.subst(var, &Term::Num { val: 0 });
        }
        zero_arm = Some((Pattern::Num(MatchNum::Zero), std::mem::take(body)));
        break;
      }
      Pattern::Num(_) => (),
      _ => unreachable!(),
    }
  }

  let mut succ_arm = None;
  for (pat, body) in arms.iter_mut() {
    match pat {
      // Var already detached.
      // match x {0: ...; +: ...}
      Pattern::Num(MatchNum::Succ(None)) => {
        let body = std::mem::take(body);
        succ_arm = Some((Pattern::Num(MatchNum::Succ(None)), body));
        break;
      }
      // Need to detach var.
      // match x {0: ...; +: @x-1 ...}
      Pattern::Num(MatchNum::Succ(Some(var))) => {
        let body = Term::Lam { tag: Tag::Static, nam: var.clone(), bod: Box::new(std::mem::take(body)) };
        succ_arm = Some((Pattern::Num(MatchNum::Succ(None)), body));
        break;
      }
      // Need to detach and increment again.
      // match x {0: ...; +: @x-1 let x = (+ x-1 1); ...}
      Pattern::Var(Some(var)) => {
        let body = Term::Let {
          pat: Pattern::Var(Some(var.clone())),
          val: Box::new(Term::Opx {
            op: Op::ADD,
            fst: Box::new(Term::Var { nam: Name::new("%pred") }),
            snd: Box::new(Term::Num { val: 1 }),
          }),
          nxt: Box::new(std::mem::take(body)),
        };

        let body = Term::named_lam(Name::new("%pred"), body);
        succ_arm = Some((Pattern::Num(MatchNum::Succ(None)), body));
        break;
      }
      // Var unused, so no need to increment
      // match x {0: ...; +: @* ...}
      Pattern::Var(None) => {
        let body = Term::erased_lam(std::mem::take(body));
        succ_arm = Some((Pattern::Num(MatchNum::Succ(None)), body));
        break;
      }
      Pattern::Num(_) => (),
      _ => unreachable!(),
    }
  }

  let Some(zero_arm) = zero_arm else {
    return Err(MatchError::Missing(["0".to_string().into()].into()));
  };
  let Some(succ_arm) = succ_arm else {
    return Err(MatchError::Missing(["+".to_string().into()].into()));
  };
  *arms = vec![zero_arm, succ_arm];
  Ok(())
}

//== Common ==//

/// Finds the expected type of the matched argument.
/// Errors on incompatible types.
/// Short-circuits if the first pattern is Type::Any.
fn infer_match_type<'a>(
  pats: impl Iterator<Item = &'a Pattern>,
  ctrs: &IndexMap<Name, Name>,
) -> Result<Type, MatchError> {
  let mut match_type = Type::None;
  for pat in pats {
    let new_type = pat.to_type(ctrs);
    match (new_type, &mut match_type) {
      (new, Type::None) => match_type = new,
      // TODO: Should we throw a type inference error in this case?
      // Since anything else will be sort of the wrong type (expected Var).
      (_, Type::Any) => (),
      (Type::Any, _) => (),
      (Type::Tup, Type::Tup) => (),
      (Type::Num, Type::Num) => (),
      (Type::Adt(nam_new), Type::Adt(nam_old)) if &nam_new == nam_old => (),
      (new, old) => {
        return Err(MatchError::Infer(format!("Type mismatch. Found '{}' expected {}.", new, old)));
      }
    };
  }
  Ok(match_type)
}

/// Converts free vars inside the match arms into lambdas with applications to give them the external value.
/// Makes the rules extractable and linear (no need for dups when variable used in both rules)
fn linearize_match_free_vars(match_term: &mut Term) -> &mut Term {
  let Term::Mat { matched: _, arms } = match_term else { unreachable!() };
  // Collect the vars.
  // We need consistent iteration order.
  let free_vars: BTreeSet<Name> = arms
    .iter()
    .flat_map(|(pat, term)| term.free_vars().into_keys().filter(|k| !pat.names().contains(k)))
    .collect();

  // Add lambdas to the arms
  for (_, body) in arms {
    let old_body = std::mem::take(body);
    *body = free_vars.iter().rev().fold(old_body, |body, var| Term::named_lam(var.clone(), body));
  }

  // Add apps to the match
  let old_match = std::mem::take(match_term);
  *match_term = free_vars.into_iter().fold(old_match, Term::arg_call);

  // Get a reference to the match again
  // It returns a reference and not an owned value because we want
  //  to keep the new surrounding Apps but still modify the match further.
  let mut match_term = match_term;
  loop {
    match match_term {
      Term::App { tag: _, fun, arg: _ } => match_term = fun.as_mut(),
      Term::Mat { .. } => return match_term,
      _ => unreachable!(),
    }
  }
}

fn linearize_match_unscoped_vars(match_term: &mut Term) -> Result<&mut Term, MatchError> {
  let Term::Mat { matched: _, arms } = match_term else { unreachable!() };
  // Collect the vars
  let mut free_vars = IndexSet::new();
  for (_, arm) in arms.iter_mut() {
    let (decls, uses) = arm.unscoped_vars();
    // Not allowed to declare unscoped var and not use it since we need to extract the match arm.
    if let Some(var) = decls.difference(&uses).next() {
      return Err(MatchError::Linearize(format!("λ${var}").into()));
    }
    // Change unscoped var to normal scoped var if it references something outside this match arm.
    let arm_free_vars = uses.difference(&decls);
    for var in arm_free_vars.clone() {
      arm.subst_unscoped(var, &Term::Var { nam: format!("%match%unscoped%{var}").into() });
    }
    free_vars.extend(arm_free_vars.cloned());
  }

  // Add lambdas to the arms
  for (_, body) in arms {
    let old_body = std::mem::take(body);
    *body = free_vars
      .iter()
      .rev()
      .fold(old_body, |body, var| Term::named_lam(format!("%match%unscoped%{var}").into(), body));
  }

  // Add apps to the match
  let old_match = std::mem::take(match_term);
  *match_term = free_vars.into_iter().fold(old_match, |acc, nam| Term::call(acc, [Term::Lnk { nam }]));

  // Get a reference to the match again
  // It returns a reference and not an owned value because we want
  //  to keep the new surrounding Apps but still modify the match further.
  let mut match_term = match_term;
  loop {
    match match_term {
      Term::App { tag: _, fun, arg: _ } => match_term = fun.as_mut(),
      Term::Mat { .. } => return Ok(match_term),
      _ => unreachable!(),
    }
  }
}
