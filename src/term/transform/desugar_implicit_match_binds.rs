use crate::term::{Adt, Book, MatchNum, Name, Pattern, Term};
use indexmap::IndexMap;

impl Book {
  pub fn desugar_implicit_match_binds(&mut self) {
    for def in self.defs.values_mut() {
      for rule in &mut def.rules {
        rule.body.desugar_implicit_match_binds(&self.ctrs, &self.adts);
      }
    }
  }
}

impl Term {
  pub fn desugar_implicit_match_binds(&mut self, ctrs: &IndexMap<Name, Name>, adts: &IndexMap<Name, Adt>) {
    let mut to_desugar = vec![self];

    while let Some(term) = to_desugar.pop() {
      match term {
        Term::Mat { matched, .. } => {
          let matched = if let Term::Var { nam } = matched.as_ref() {
            nam.clone()
          } else {
            let Term::Mat { matched, arms } = std::mem::take(term) else { unreachable!() };

            let nam = Name::new("%matched");

            *term = Term::Let {
              pat: Pattern::Var(Some(nam.clone())),
              val: matched,
              nxt: Box::new(Term::Mat { matched: Box::new(Term::Var { nam: nam.clone() }), arms }),
            };

            nam
          };

          let (Term::Mat { arms, .. } | Term::Let { nxt: box Term::Mat { arms, .. }, .. }) = term else {
            unreachable!()
          };

          for (pat, body) in arms {
            match pat {
              Pattern::Var(_) => (),
              Pattern::Ctr(nam, pat_args) => {
                let adt = ctrs.get(nam).unwrap();
                let Adt { ctrs, .. } = adts.get(adt).unwrap();
                let ctr_args = ctrs.get(nam).unwrap();
                if pat_args.is_empty() && !ctr_args.is_empty() {
                  // Implicit ctr args
                  *pat_args = ctr_args
                    .iter()
                    .map(|field| Pattern::Var(Some(format!("{matched}.{field}").into())))
                    .collect();
                }
              }
              Pattern::Num(MatchNum::Zero) => (),
              Pattern::Num(MatchNum::Succ(Some(_))) => (),
              Pattern::Num(MatchNum::Succ(p @ None)) => {
                // Implicit num arg
                *p = Some(Some(format!("{matched}-1").into()));
              }
              Pattern::Tup(_, _) => (),
              Pattern::Lst(..) => unreachable!(),
            }
            to_desugar.push(body);
          }
        }
        Term::Let { pat: Pattern::Var(_), val: fst, nxt: snd }
        | Term::App { fun: fst, arg: snd, .. }
        | Term::Dup { val: fst, nxt: snd, .. }
        | Term::Tup { fst, snd }
        | Term::Sup { fst, snd, .. }
        | Term::Opx { fst, snd, .. } => {
          to_desugar.push(fst);
          to_desugar.push(snd);
        }
        Term::Lam { bod, .. } | Term::Chn { bod, .. } => to_desugar.push(bod),
        Term::Era
        | Term::Ref { .. }
        | Term::Num { .. }
        | Term::Str { .. }
        | Term::Lnk { .. }
        | Term::Var { .. }
        | Term::Err => (),
        Term::Let { pat: _, .. } => {
          unreachable!("Expected destructor let expressions to have been desugared already")
        }
        Term::Lst { .. } => unreachable!("Should have been desugared already"),
      }
    }
  }
}
