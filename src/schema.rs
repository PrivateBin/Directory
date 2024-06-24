// @generated automatically by Diesel CLI.

diesel::table! {
    checks (id) {
        id -> Integer,
        updated -> Timestamp,
        up -> Bool,
        instance_id -> Integer,
    }
}

diesel::table! {
    instances (id) {
        id -> Integer,
        url -> Text,
        version -> Text,
        https -> Bool,
        https_redirect -> Bool,
        country_id -> Text,
        attachments -> Bool,
        csp_header -> Bool,
        variant -> Integer,
    }
}

diesel::table! {
    scans (id) {
        id -> Integer,
        scanner -> Text,
        rating -> Text,
        percent -> Integer,
        instance_id -> Integer,
    }
}

diesel::joinable!(checks -> instances (instance_id));
diesel::joinable!(scans -> instances (instance_id));

diesel::allow_tables_to_appear_in_same_query!(checks, instances, scans,);
