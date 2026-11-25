#[macro_export]
macro_rules! arithmetic_conds {
    ($in_sig:ident $op:tt $second:expr => $out_sig:ident) => {
        $crate::blueprint::models::ArithmeticCombinatorParameters {
            first_signal:           ::std::option::Option::Some($in_sig.clone()),
            first_signal_networks:  "both".into(),
            second_signal:          ::std::option::Option::None,
            second_signal_networks: "both".into(),
            second_constant:        $second,
            operation:              $crate::blueprint::macros::arithmetic_cond_op!($op),
            output_signal:          ::std::option::Option::Some($out_sig.clone()),
        }
    };
}

#[macro_export]
macro_rules! arithmetic_cond_op {
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

#[macro_export]
macro_rules! decider_cond {
    ($sig_a:expr, $op:tt, $sig_b:expr) => {
        $crate::blueprint::models::DeciderCombinatorCondition::new(
            $sig_a,
            $sig_b,
            $crate::blueprint::macros::decider_cond_op!($op),
        )
    };
}

#[macro_export]
macro_rules! decider_cond_op {
    (==) => {
        $crate::blueprint::models::ComparatorString::Eq
    };
    (>) => {
        $crate::blueprint::models::ComparatorString::Gt
    };
    (<) => {
        $crate::blueprint::models::ComparatorString::Lt
    };
    (>=) => {
        $crate::blueprint::models::ComparatorString::Ge
    };
    (<=) => {
        $crate::blueprint::models::ComparatorString::Le
    };
    (!=) => {
        $crate::blueprint::models::ComparatorString::Ne
    };
}

/// High-level utility macro for making wire connections.
#[macro_export]
macro_rules! mkwires {
    ($wire_type:ident $ents:ident; $src_type:ident $src_name:expr => $dst_type:ident $dst_name:expr) => {
        [
            $crate::blueprint::macros::entity_idx_by_tag!($ents, $src_name).expect("No entity found with tag"),
            $crate::blueprint::macros::mkwires!(@wire_type $wire_type, $src_type),
            $crate::blueprint::macros::entity_idx_by_tag!($ents, $dst_name).expect("No entity found with tag"),
            $crate::blueprint::macros::mkwires!(@wire_type $wire_type, $dst_type),
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
        $ents.iter().find(|e| e.has_custom_tag($tag)).map(|e| e.entity_number)
    };
}

pub use arithmetic_cond_op;
pub use decider_cond_op;
pub use entity_idx_by_tag;
pub use mkwires;
