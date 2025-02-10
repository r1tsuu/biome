use biome_analyze::ActionCategory;
use biome_analyze::SourceActionKind;
use biome_rowan::Direction;
use biome_rowan::TextRange;
use std::borrow::Cow;
use std::cmp::Ordering;

use biome_analyze::{context::RuleContext, declare_source_rule, Ast, Rule, RuleAction};
use biome_console::markup;
use biome_deserialize_macros::Deserializable;
use biome_diagnostics::Applicability;
use biome_js_syntax::{AnyJsObjectMember, AnyJsObjectMemberName, JsObjectMemberList};
use biome_rowan::{AstNode, AstSeparatedList, BatchMutationExt, SyntaxResult};
use serde::{Deserialize, Serialize};

use crate::JsRuleAction;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, Deserializable)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct UseSortedPropertiesOptions {
    /// Partition the object properties by a newline.
    pub partion_by_new_line: bool,
    /// Partition the object properties by a comment.
    pub partion_by_comment: bool,
}

declare_source_rule! {
    /// Enforce keys sorting in objects.
    ///
    /// This rule checks if keys of the objects are sorted in a consistent way.
    /// Keys are sorted alphabetically.
    /// This rule will consider spread/calculated keys e.g [k]: 1 as non-sortable.
    /// Instead, whenever it encounters a non-sortable key, it will sort all the
    /// previous sortable keys up until the nearest non-sortable key, if one
    /// exist.
    /// This prevents breaking the override of certain keys using spread
    /// keys.
    ///
    /// Source: https://perfectionist.dev/rules/sort-objects
    ///
    /// ## Examples
    ///
    /// ```js,expect_diff
    /// {
    ///   x: 1,
    ///   a: 2,
    /// };
    /// ```
    ///
    /// ```js,expect_diff
    /// {
    ///   x: 1,
    ///   ...f,
    ///   y: 4,
    ///   a: 2,
    ///   [calculated()]: true,
    ///   b: 3,
    ///   a: 1,
    /// };
    /// ```
    ///
    pub UseSortedProperties {
        version: "1.0.0",
        name: "useSortedProperties",
        language: "js",
        recommended: false,
    }
}

impl Rule for UseSortedProperties {
    type Query = Ast<JsObjectMemberList>;
    type State = Vec<Vec<ObjectMember>>;
    type Signals = Option<Self::State>;
    type Options = UseSortedPropertiesOptions;

    fn run(ctx: &RuleContext<Self>) -> Self::Signals {
        let mut members = vec![];
        let mut groups = vec![];

        let get_name = |name: SyntaxResult<AnyJsObjectMemberName>| {
            if let Ok(name) = name {
                if let Some(name) = name.as_js_literal_member_name() {
                    return Some(name.text());
                }
            }

            None
        };

        for (element_index, element) in ctx.query().elements().enumerate() {
            if let Ok(element) = element.node() {
                let mut create_group = false;
                println!("Element: {:?}", element_index);
                // Handle partitioning by comment or newline if enabled
                if element_index != 0 && ctx.options().partion_by_comment
                    || ctx.options().partion_by_new_line
                {
                    println!("Partitioning by comment or newline");
                    'outer: for token in element.syntax().descendants_tokens(Direction::Prev) {
                        let mut newline_count = 0;
                        for piece in token.leading_trivia().pieces() {
                            // Handle comment partitioning
                            if piece.is_comments() && ctx.options().partion_by_comment {
                                create_group = true;
                                break 'outer;
                            }

                            // Handle newline partitioning
                            if piece.is_newline() && ctx.options().partion_by_new_line {
                                newline_count += 1;

                                if newline_count == 2 {
                                    create_group = true;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }

                if create_group {
                    groups.push(members.clone());
                    members.clear();
                }

                match element {
                    AnyJsObjectMember::JsSpread(_) | AnyJsObjectMember::JsBogusMember(_) => {
                        members.push(ObjectMember::new(element.clone(), None));
                    }
                    AnyJsObjectMember::JsPropertyObjectMember(member) => {
                        members.push(ObjectMember::new(element.clone(), get_name(member.name())));
                    }
                    AnyJsObjectMember::JsGetterObjectMember(member) => {
                        members.push(ObjectMember::new(element.clone(), get_name(member.name())));
                    }
                    AnyJsObjectMember::JsSetterObjectMember(member) => {
                        members.push(ObjectMember::new(element.clone(), get_name(member.name())));
                    }
                    AnyJsObjectMember::JsMethodObjectMember(member) => {
                        members.push(ObjectMember::new(element.clone(), get_name(member.name())));
                    }
                    AnyJsObjectMember::JsShorthandPropertyObjectMember(member) => {
                        members.push(ObjectMember::new(element.clone(), Some(member.text())));
                    }
                }
            }
        }

        if !members.is_empty() {
            groups.push(members);
        }

        Some(groups)
    }

    fn action(ctx: &RuleContext<Self>, state: &Self::State) -> Option<JsRuleAction> {
        let mut sorted_state = state.clone();
        sorted_state.iter_mut().for_each(|group| group.sort());

        if sorted_state == *state {
            return None;
        }

        let mut mutation = ctx.root().begin();

        for (unsorted, sorted) in state.iter().flatten().zip(sorted_state.iter().flatten()) {
            mutation.replace_node(unsorted.member.clone(), sorted.member.clone());
        }

        Some(RuleAction::new(
            rule_action_category!(),
            Applicability::Always,
            markup! { "Sort the object properties." },
            mutation,
        ))
    }
}

#[derive(PartialEq, Eq, Clone)]
pub struct ObjectMember {
    member: AnyJsObjectMember,
    name: Option<String>,
}

impl ObjectMember {
    fn new(member: AnyJsObjectMember, name: Option<String>) -> Self {
        ObjectMember { member, name }
    }
}

impl Ord for ObjectMember {
    fn cmp(&self, other: &Self) -> Ordering {
        let (Some(self_name), Some(other_name)) = (&self.name, &other.name) else {
            return Ordering::Equal;
        };

        natord::compare(&self_name, &other_name)
    }
}

impl PartialOrd for ObjectMember {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
