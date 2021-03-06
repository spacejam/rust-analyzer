//! Many kinds of items or constructs can have generic parameters: functions,
//! structs, impls, traits, etc. This module provides a common HIR for these
//! generic parameters. See also the `Generics` type and the `generics_of` query
//! in rustc.

use std::sync::Arc;

use ra_syntax::ast::{self, NameOwner, TypeParamsOwner, TypeBoundsOwner};

use crate::{
    db::DefDatabase,
    Name, AsName, Function, Struct, Enum, Trait, TypeAlias, ImplBlock, Container, path::Path, type_ref::TypeRef, AdtDef
};

/// Data about a generic parameter (to a function, struct, impl, ...).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GenericParam {
    // FIXME: give generic params proper IDs
    pub(crate) idx: u32,
    pub(crate) name: Name,
}

/// Data about the generic parameters of a function, struct, impl, etc.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GenericParams {
    pub(crate) parent_params: Option<Arc<GenericParams>>,
    pub(crate) params: Vec<GenericParam>,
    pub(crate) where_predicates: Vec<WherePredicate>,
}

/// A single predicate from a where clause, i.e. `where Type: Trait`. Combined
/// where clauses like `where T: Foo + Bar` are turned into multiple of these.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WherePredicate {
    type_ref: TypeRef,
    trait_ref: Path,
}

// FIXME: consts can have type parameters from their parents (i.e. associated consts of traits)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum GenericDef {
    Function(Function),
    Struct(Struct),
    Enum(Enum),
    Trait(Trait),
    TypeAlias(TypeAlias),
    ImplBlock(ImplBlock),
}
impl_froms!(GenericDef: Function, Struct, Enum, Trait, TypeAlias, ImplBlock);

impl GenericParams {
    pub(crate) fn generic_params_query(
        db: &impl DefDatabase,
        def: GenericDef,
    ) -> Arc<GenericParams> {
        let mut generics = GenericParams::default();
        let parent = match def {
            GenericDef::Function(it) => it.container(db).map(GenericDef::from),
            GenericDef::TypeAlias(it) => it.container(db).map(GenericDef::from),
            GenericDef::Struct(_) | GenericDef::Enum(_) | GenericDef::Trait(_) => None,
            GenericDef::ImplBlock(_) => None,
        };
        generics.parent_params = parent.map(|p| db.generic_params(p));
        let start = generics.parent_params.as_ref().map(|p| p.params.len()).unwrap_or(0) as u32;
        match def {
            GenericDef::Function(it) => generics.fill(&*it.source(db).1, start),
            GenericDef::Struct(it) => generics.fill(&*it.source(db).1, start),
            GenericDef::Enum(it) => generics.fill(&*it.source(db).1, start),
            GenericDef::Trait(it) => {
                // traits get the Self type as an implicit first type parameter
                generics.params.push(GenericParam { idx: start, name: Name::self_type() });
                generics.fill(&*it.source(db).1, start + 1);
            }
            GenericDef::TypeAlias(it) => generics.fill(&*it.source(db).1, start),
            GenericDef::ImplBlock(it) => generics.fill(&*it.source(db).1, start),
        }

        Arc::new(generics)
    }

    fn fill(&mut self, node: &impl TypeParamsOwner, start: u32) {
        if let Some(params) = node.type_param_list() {
            self.fill_params(params, start)
        }
        if let Some(where_clause) = node.where_clause() {
            self.fill_where_predicates(where_clause);
        }
    }

    fn fill_params(&mut self, params: &ast::TypeParamList, start: u32) {
        for (idx, type_param) in params.type_params().enumerate() {
            let name = type_param.name().map(AsName::as_name).unwrap_or_else(Name::missing);
            let param = GenericParam { idx: idx as u32 + start, name };
            self.params.push(param);
        }
    }

    fn fill_where_predicates(&mut self, where_clause: &ast::WhereClause) {
        for pred in where_clause.predicates() {
            let type_ref = match pred.type_ref() {
                Some(type_ref) => type_ref,
                None => continue,
            };
            for bound in pred.type_bound_list().iter().flat_map(|l| l.bounds()) {
                let path = bound
                    .type_ref()
                    .and_then(|tr| match tr.kind() {
                        ast::TypeRefKind::PathType(path) => path.path(),
                        _ => None,
                    })
                    .and_then(Path::from_ast);
                let path = match path {
                    Some(p) => p,
                    None => continue,
                };
                self.where_predicates.push(WherePredicate {
                    type_ref: TypeRef::from_ast(type_ref),
                    trait_ref: path,
                });
            }
        }
    }

    pub(crate) fn find_by_name(&self, name: &Name) -> Option<&GenericParam> {
        self.params.iter().find(|p| &p.name == name)
    }

    pub fn count_parent_params(&self) -> usize {
        self.parent_params.as_ref().map(|p| p.count_params_including_parent()).unwrap_or(0)
    }

    pub fn count_params_including_parent(&self) -> usize {
        let parent_count = self.count_parent_params();
        parent_count + self.params.len()
    }

    fn for_each_param<'a>(&'a self, f: &mut impl FnMut(&'a GenericParam)) {
        if let Some(parent) = &self.parent_params {
            parent.for_each_param(f);
        }
        self.params.iter().for_each(f);
    }

    pub fn params_including_parent(&self) -> Vec<&GenericParam> {
        let mut vec = Vec::with_capacity(self.count_params_including_parent());
        self.for_each_param(&mut |p| vec.push(p));
        vec
    }
}

impl From<Container> for GenericDef {
    fn from(c: Container) -> Self {
        match c {
            Container::Trait(trait_) => trait_.into(),
            Container::ImplBlock(impl_block) => impl_block.into(),
        }
    }
}

impl From<crate::adt::AdtDef> for GenericDef {
    fn from(adt: crate::adt::AdtDef) -> Self {
        match adt {
            AdtDef::Struct(s) => s.into(),
            AdtDef::Enum(e) => e.into(),
        }
    }
}

pub trait HasGenericParams {
    fn generic_params(self, db: &impl DefDatabase) -> Arc<GenericParams>;
}

impl<T> HasGenericParams for T
where
    T: Into<GenericDef>,
{
    fn generic_params(self, db: &impl DefDatabase) -> Arc<GenericParams> {
        db.generic_params(self.into())
    }
}
