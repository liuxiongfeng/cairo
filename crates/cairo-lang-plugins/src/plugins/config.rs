use std::sync::Arc;

use cairo_lang_defs::plugin::{MacroPlugin, PluginResult};
use cairo_lang_filesystem::cfg::{Cfg, CfgSet};
use cairo_lang_semantic::plugin::{AsDynMacroPlugin, SemanticPlugin};
use cairo_lang_syntax::node::ast::AttributeArgs;
use cairo_lang_syntax::node::db::SyntaxGroup;
use cairo_lang_syntax::node::{ast, Terminal, TypedSyntaxNode};

/// Plugin that enables ignoring modules not involved in the current config.
/// Mostly useful for marking test modules to prevent usage of their functionality out of tests,
/// and reduce compilation time when the tests data isn't required.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ConfigPlugin;

impl MacroPlugin for ConfigPlugin {
    fn generate_code(&self, db: &dyn SyntaxGroup, item_ast: ast::Item) -> PluginResult {
        let item_attributes = match item_ast {
            ast::Item::Constant(ast_node) => ast_node.attributes(db),
            ast::Item::Module(ast_node) => ast_node.attributes(db),
            ast::Item::Use(ast_node) => ast_node.attributes(db),
            ast::Item::FreeFunction(ast_node) => ast_node.attributes(db),
            ast::Item::ExternFunction(ast_node) => ast_node.attributes(db),
            ast::Item::ExternType(ast_node) => ast_node.attributes(db),
            ast::Item::Trait(ast_node) => ast_node.attributes(db),
            ast::Item::Impl(ast_node) => ast_node.attributes(db),
            ast::Item::Struct(ast_node) => ast_node.attributes(db),
            ast::Item::Enum(ast_node) => ast_node.attributes(db),
            ast::Item::TypeAlias(ast_node) => ast_node.attributes(db),
        };
        let cfg_set = db.cfg_set();
        for attr in item_attributes.elements(db) {
            if attr.attr(db).text(db) == "cfg" {
                if let ast::OptionAttributeArgs::AttributeArgs(args) = attr.args(db) {
                    let pattern = parse_predicate(db, args);
                    if !cfg_set.is_superset(&pattern) {
                        return PluginResult {
                            code: None,
                            diagnostics: vec![],
                            remove_original_item: true,
                        };
                    }
                }
            }
        }
        PluginResult::default()
    }
}
impl AsDynMacroPlugin for ConfigPlugin {
    fn as_dyn_macro_plugin<'a>(self: Arc<Self>) -> Arc<dyn MacroPlugin + 'a>
    where
        Self: 'a,
    {
        self
    }
}
impl SemanticPlugin for ConfigPlugin {}

/// Parse `#[cfg(...)]` attribute arguments as a predicate matching [`Cfg`] items.
fn parse_predicate(db: &dyn SyntaxGroup, args: AttributeArgs) -> CfgSet {
    // TODO(mkaput): Support more complex expressions.
    CfgSet::from_iter([Cfg::tag(args.arg_list(db).as_syntax_node().get_text(db).trim())])
}
