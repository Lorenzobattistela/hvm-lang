use super::{parser::parse_book, Book, Name, Pattern, Term};
use hvmc::run::Val;

const BUILTINS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/term/builtins.hvm"));

pub const LIST: &str = "List";
pub const LCONS: &str = "List.cons";
pub const LNIL: &str = "List.nil";

pub const HEAD: &str = "head";
pub const TAIL: &str = "tail";

pub const STRING: &str = "String";
pub const SCONS: &str = "String.cons";
pub const SNIL: &str = "String.nil";

impl Book {
  pub fn builtins() -> Book {
    parse_book(BUILTINS, Book::default, true).expect("Error parsing builtin file, this should not happen")
  }

  pub fn encode_builtins(&mut self) {
    for def in self.defs.values_mut() {
      for rule in def.rules.iter_mut() {
        rule.pats.iter_mut().for_each(Pattern::encode_builtins);
        rule.body.encode_builtins();
      }
    }
  }
}

impl Term {
  fn encode_builtins(&mut self) {
    match self {
      Term::Lst { els } => *self = Term::encode_list(std::mem::take(els)),
      Term::Str { val } => *self = Term::encode_str(val),
      Term::Let { pat, val, nxt } => {
        pat.encode_builtins();
        val.encode_builtins();
        nxt.encode_builtins();
      }
      Term::Lam { bod, .. } | Term::Chn { bod, .. } => bod.encode_builtins(),
      Term::App { fun: fst, arg: snd, .. }
      | Term::Tup { fst, snd }
      | Term::Sup { fst, snd, .. }
      | Term::Opx { fst, snd, .. }
      | Term::Dup { val: fst, nxt: snd, .. } => {
        fst.encode_builtins();
        snd.encode_builtins();
      }
      Term::Mat { matched, arms } => {
        matched.encode_builtins();
        for (pat, arm) in arms {
          pat.encode_builtins();
          arm.encode_builtins();
        }
      }
      Term::Var { .. } | Term::Lnk { .. } | Term::Ref { .. } | Term::Num { .. } | Term::Era | Term::Err => {}
    }
  }

  fn encode_list(elements: Vec<Term>) -> Term {
    elements.into_iter().rfold(Term::r#ref(LNIL), |acc, mut nxt| {
      nxt.encode_builtins();
      Term::call(Term::r#ref(LCONS), [nxt, acc])
    })
  }

  fn encode_str(val: &str) -> Term {
    val.chars().rfold(Term::r#ref(SNIL), |acc, char| {
      Term::call(Term::r#ref(SCONS), [Term::Num { val: Val::from(char) }, acc])
    })
  }
}

impl Pattern {
  pub fn encode_builtins(&mut self) {
    match self {
      Pattern::Lst(pats) => *self = Self::encode_list(std::mem::take(pats)),
      Pattern::Ctr(_, pats) => {
        for pat in pats {
          pat.encode_builtins();
        }
      }
      Pattern::Tup(fst, snd) => {
        fst.encode_builtins();
        snd.encode_builtins();
      }
      Pattern::Var(..) | Pattern::Num(..) => {}
    }
  }

  fn encode_list(elements: Vec<Pattern>) -> Pattern {
    let lnil = Pattern::Var(Some(Name::new(LNIL)));

    elements.into_iter().rfold(lnil, |acc, mut nxt| {
      nxt.encode_builtins();
      Pattern::Ctr(Name::new(LCONS), vec![nxt, acc])
    })
  }
}
