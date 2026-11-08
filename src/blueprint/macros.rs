#[macro_export]
macro_rules! arithmetic_conds {
    ($in_sig:ident $op:tt $second:expr => $out_sig:ident) => {
        $crate::blueprint::models::ArithmeticCombinatorParameters {
            first_signal:    Some(std::sync::Arc::clone(&$in_sig)),
            second_signal:   None,
            second_constant: $second,
            operation:       $crate::blueprint::macros::arithmetic_combinator_op!($op),
            output_signal:   Some(std::sync::Arc::clone(&$out_sig)),
        }
    };
}

#[macro_export]
macro_rules! arithmetic_combinator_op {
    (*) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Mul
    };
    (/) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Div
    };
    (+) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Add
    };
    (-) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Sub
    };
    (%) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Mod
    };
    (^) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Pow
    };
    (<<) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Shl
    };
    (>>) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Shr
    };
    (AND) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::And
    };
    (OR) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Or
    };
    (XOR) => {
        $crate::blueprint::models::ArithmeticCombinatorParameterOperation::Xor
    };
}

/// High-level utility macro for making wire connections.
#[macro_export]
macro_rules! mkwires {
    ($wire_type:ident $ents:ident; $src_type:ident $src_name:expr => $dst_type:ident $dst_name:expr) => {
        [
            entity_idx_by_tag!($ents, $src_name).expect("No entity found with tag"),
            mkwires!(@wire_type $wire_type, $src_type),
            entity_idx_by_tag!($ents, $dst_name).expect("No entity found with tag"),
            mkwires!(@wire_type $wire_type, $dst_type),
        ]
    };

    (@wire_type R, IN) => { 1 };
    (@wire_type G, IN) => { 2 };
    (@wire_type R, OUT) => { 3 };
    (@wire_type G, OUT) => { 4 };

    (@wire_type C, IN) => { 5 };
    (@wire_type C, OUT) => { 6 };
}

#[macro_export]
macro_rules! entity_idx_by_tag {
    ($ents:ident, $tag:expr) => {
        $ents.iter().find(|e| e.has_tag($tag.to_string())).map(|e| e.entity_number)
    };
}

pub use arithmetic_combinator_op;
pub use arithmetic_conds;
pub use entity_idx_by_tag;
pub use mkwires;
