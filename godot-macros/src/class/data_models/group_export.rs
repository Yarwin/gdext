/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Here I shall explain how groups in Godot works

use crate::class::data_models::fields::Fields;
use crate::util::KvParser;
use std::cmp::Ordering;

/// Points to index of a given group name in [Fields.groups](field@Fields::groups).
///
/// Two fields with the same GroupIdentifier belong to the same group.
pub type GroupIdentifier = usize;

pub struct FieldGroup {
    pub group_name_index: Option<GroupIdentifier>,
    pub subgroup_name_index: Option<GroupIdentifier>,
}

impl FieldGroup {
    fn parse_group(
        expr: &'static str,
        parser: &mut KvParser,
        groups: &mut Vec<String>,
    ) -> Option<GroupIdentifier> {
        let group = parser.handle_expr(expr).unwrap_or(None)?.to_string();

        if let Some(group_index) = groups
            .iter()
            .position(|existing_group| existing_group == &group)
        {
            Some(group_index)
        } else {
            groups.push(group);
            Some(groups.len() - 1)
        }
    }

    pub(crate) fn new_from_kv(parser: &mut KvParser, groups: &mut Vec<String>) -> Self {
        Self {
            group_name_index: Self::parse_group("group", parser, groups),
            subgroup_name_index: Self::parse_group("subgroup", parser, groups),
        }
    }
}

// ----------------------------------------------------------------------------------------------------------------------------------------------
// Ordering

pub(crate) struct ExportGroupOrdering {
    /// Allows to identify given export group.
    /// `None` for root.
    identifier: Option<GroupIdentifier>,
    /// Contains subgroups of given ordering (subgroups for groups, subgroups&groups for root).
    /// Ones parsed first have higher priority, i.e. are displayed as the first.
    subgroups: Vec<ExportGroupOrdering>,
}

impl ExportGroupOrdering {
    fn root() -> Self {
        Self {
            identifier: None,
            subgroups: Vec::new(),
        }
    }

    /// Creates an ExportGroupOrdering belonging to given group.
    fn child(identifier: GroupIdentifier) -> Self {
        Self {
            identifier: Some(identifier),
            subgroups: Vec::new(),
        }
    }

    fn priority(&mut self, identifier: &GroupIdentifier) -> usize {
        self.subgroups
            .iter()
            .position(|sub| sub.identifier.as_ref().unwrap() == identifier)
            .unwrap_or_else(|| {
                self.subgroups.push(ExportGroupOrdering::child(*identifier));
                self.subgroups.len() - 1
            })
    }
}

// Note: GDExtension doesn't support categories for some reason(s?).
// It probably expects us to use inheritance instead?
enum OrderingStage {
    Group,
    SubGroup,
}

// It is recursive but max recursion depth is 1 so it's fine.
fn sort_by_group_priority(
    field_a: &FieldGroup,
    field_b: &FieldGroup,
    ordering: &mut ExportGroupOrdering,
    stage: OrderingStage,
) -> Ordering {
    let (lhs, rhs, next_stage) = match stage {
        OrderingStage::Group => (
            &field_a.group_name_index,
            &field_b.group_name_index,
            Some(OrderingStage::SubGroup),
        ),
        OrderingStage::SubGroup => (
            &field_a.subgroup_name_index,
            &field_b.subgroup_name_index,
            None,
        ),
    };

    match (lhs, rhs) {
        // Ungrouped fields or fields with subgroup only always have higher priority.
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        
        (Some(idx_a), Some(idx_b)) => {
            if idx_a == idx_b {
                // Fields belong to the same subgroup.
                let Some(next_stage) = next_stage else {return Ordering::Equal};

                let next_ordering_position = ordering
                    .subgroups
                    .iter_mut()
                    // Unreachable – non-root orderings must have an identifier.
                    .position(|e| e.identifier.as_ref().expect("Tried to parse undefined export group. This is a bug, please report it.") == idx_a)
                    .unwrap_or_else(|| {
                        ordering.subgroups.push(ExportGroupOrdering::child(*idx_a));
                        ordering.subgroups.len() - 1
                    });

                sort_by_group_priority(
                    field_a,
                    field_b,
                    &mut ordering.subgroups[next_ordering_position],
                    next_stage,
                )
            } else {
                let (priority_a, priority_b) = (ordering.priority(idx_a), ordering.priority(idx_b));
                priority_a.cmp(&priority_b)
            }
        }
        
        (None, None) => {
            // Fields don't belong to any subgroup nor group.
            let Some(next_stage) = next_stage else {return Ordering::Equal};
            
            sort_by_group_priority(field_a, field_b, ordering, next_stage)
        }
    }
}

/// Sorts fields by their group and subgroup association.
/// 
/// Fields without group or subgroup are first.
/// Fields with subgroup only come in next, in order of their declaration on the class struct.
/// Finally fields with groups are displayed – firstly ones without subgroups followed by 
/// fields with given group & subgroup. 
pub(crate) fn sort_fields_by_group(fields: &mut Fields) {
    fields.format_groups();
    let mut initial_ordering = ExportGroupOrdering::root();

    // `sort_by` instead of `sort_unstable_by` to preserve original order of declaration.
    // Which is not guaranteed but so far worked reliably.
    fields.all_fields.sort_unstable_by(|a, b| {
        let (group_a, group_b) = match (&a.group, &b.group) {
            (Some(a), Some(b)) => (a, b),
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            // We don't care about ordering of non-exported fields.
            _ => return Ordering::Equal,
        };

        sort_by_group_priority(
            &group_a,
            &group_b,
            &mut initial_ordering,
            OrderingStage::Group,
        )
    });
}
