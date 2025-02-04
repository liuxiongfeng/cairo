use std::sync::Arc;

use cairo_lang_diagnostics::Maybe;
use cairo_lang_filesystem::ids::CrateId;
use cairo_lang_lowering::db::LoweringGroup;
use cairo_lang_lowering::panic::PanicSignatureInfo;
use cairo_lang_sierra::extensions::lib_func::SierraApChange;
use cairo_lang_sierra::extensions::{ConcreteType, GenericTypeEx};
use cairo_lang_sierra::ids::ConcreteTypeId;
use cairo_lang_utils::Upcast;
use lowering::ids::ConcreteFunctionWithBodyId;
use semantic::Mutability;
use {cairo_lang_lowering as lowering, cairo_lang_semantic as semantic};

use crate::program_generator::{self};
use crate::specialization_context::SierraSignatureSpecializationContext;
use crate::{ap_change, function_generator, pre_sierra};

#[salsa::query_group(SierraGenDatabase)]
pub trait SierraGenGroup: LoweringGroup + Upcast<dyn LoweringGroup> {
    #[salsa::interned]
    fn intern_label_id(&self, id: pre_sierra::LabelLongId) -> pre_sierra::LabelId;

    #[salsa::interned]
    fn intern_concrete_lib_func(
        &self,
        id: cairo_lang_sierra::program::ConcreteLibfuncLongId,
    ) -> cairo_lang_sierra::ids::ConcreteLibfuncId;

    #[salsa::interned]
    fn intern_concrete_type(
        &self,
        id: cairo_lang_sierra::program::ConcreteTypeLongId,
    ) -> cairo_lang_sierra::ids::ConcreteTypeId;

    /// Creates a Sierra function id for a function id of the semantic model.
    // TODO(lior): Can we have the short and long ids in the same place? Currently, the short
    //   id is defined in sierra and the long id is defined in semantic.
    #[salsa::interned]
    fn intern_sierra_function(
        &self,
        id: lowering::ids::FunctionId,
    ) -> cairo_lang_sierra::ids::FunctionId;

    /// Returns the matching sierra concrete type id for a given semantic type id.
    #[salsa::invoke(crate::types::get_concrete_type_id)]
    fn get_concrete_type_id(
        &self,
        type_id: semantic::TypeId,
    ) -> Maybe<cairo_lang_sierra::ids::ConcreteTypeId>;

    /// Returns the [cairo_lang_sierra::program::FunctionSignature] object for the given function
    /// id.
    fn get_function_signature(
        &self,
        function_id: cairo_lang_sierra::ids::FunctionId,
    ) -> Maybe<Arc<cairo_lang_sierra::program::FunctionSignature>>;

    /// Returns the [cairo_lang_sierra::extensions::types::TypeInfo] object for the given type id.
    fn get_type_info(
        &self,
        concrete_type_id: cairo_lang_sierra::ids::ConcreteTypeId,
    ) -> Maybe<Arc<cairo_lang_sierra::extensions::types::TypeInfo>>;

    /// Private query to compute Sierra data about a function with body.
    #[salsa::invoke(function_generator::priv_function_with_body_sierra_data)]
    fn priv_function_with_body_sierra_data(
        &self,
        function_id: ConcreteFunctionWithBodyId,
    ) -> function_generator::SierraFunctionWithBodyData;
    /// Returns the Sierra code (as [pre_sierra::Function]) for a given function with body.
    #[salsa::invoke(function_generator::function_with_body_sierra)]
    fn function_with_body_sierra(
        &self,
        function_id: ConcreteFunctionWithBodyId,
    ) -> Maybe<Arc<pre_sierra::Function>>;

    /// Returns the ap change of a given function if it is known at compile time or
    /// [SierraApChange::Unknown] otherwise.
    #[salsa::invoke(ap_change::get_ap_change)]
    fn get_ap_change(&self, function_id: ConcreteFunctionWithBodyId) -> Maybe<SierraApChange>;

    /// Returns the [cairo_lang_sierra::program::Program] object of the requested functions.
    #[salsa::invoke(program_generator::get_sierra_program_for_functions)]
    fn get_sierra_program_for_functions(
        &self,
        requested_function_ids: Vec<ConcreteFunctionWithBodyId>,
    ) -> Maybe<Arc<cairo_lang_sierra::program::Program>>;

    /// Returns the [cairo_lang_sierra::program::Program] object of the requested crates.
    #[salsa::invoke(program_generator::get_sierra_program)]
    fn get_sierra_program(
        &self,
        requested_crate_ids: Vec<CrateId>,
    ) -> Maybe<Arc<cairo_lang_sierra::program::Program>>;
}

fn get_function_signature(
    db: &dyn SierraGenGroup,
    function_id: cairo_lang_sierra::ids::FunctionId,
) -> Maybe<Arc<cairo_lang_sierra::program::FunctionSignature>> {
    // TODO(yuval): add another version of this function that directly received semantic FunctionId.
    // Call it from function_generators::get_function_code. Take ret_types from the result instead
    // of only the explicit ret_type. Also use it for params instead of the current logic. Then use
    // it in the end of program_generator::get_sierra_program instead of calling this function from
    // there.
    let semantic_function_id = db.lookup_intern_sierra_function(function_id);
    let signature = semantic_function_id.signature(db.upcast())?;
    let may_panic = db.function_may_panic(semantic_function_id)?;

    let implicits = db
        .function_implicits(semantic_function_id)?
        .iter()
        .map(|ty| db.get_concrete_type_id(*ty))
        .collect::<Maybe<Vec<ConcreteTypeId>>>()?;

    // TODO(spapini): Handle ret_types in lowering.
    let mut all_params = implicits.clone();
    let mut ref_types = vec![];
    for param in &signature.params {
        let concrete_type_id = db.get_concrete_type_id(param.ty)?;
        all_params.push(concrete_type_id.clone());
        if param.mutability == Mutability::Reference {
            ref_types.push(concrete_type_id);
        }
    }

    // TODO(ilya): Handle tuple and struct types.
    let mut ret_types = implicits;
    if may_panic {
        let panic_info = PanicSignatureInfo::new(db.upcast(), &signature);
        ret_types.push(db.get_concrete_type_id(panic_info.panic_ty)?);
    } else {
        ret_types.extend(ref_types.into_iter());
        ret_types.push(db.get_concrete_type_id(signature.return_type)?);
    }

    Ok(Arc::new(cairo_lang_sierra::program::FunctionSignature {
        param_types: all_params,
        ret_types,
    }))
}

fn get_type_info(
    db: &dyn SierraGenGroup,
    concrete_type_id: cairo_lang_sierra::ids::ConcreteTypeId,
) -> Maybe<Arc<cairo_lang_sierra::extensions::types::TypeInfo>> {
    let long_id = db.lookup_intern_concrete_type(concrete_type_id);
    let concrete_ty = cairo_lang_sierra::extensions::core::CoreType::specialize_by_id(
        &SierraSignatureSpecializationContext(db),
        &long_id.generic_id,
        &long_id.generic_args,
    )
    .expect("Got failure while specializing type.");
    Ok(Arc::new(concrete_ty.info().clone()))
}
