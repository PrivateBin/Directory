table! {
    instances (id) {
        id -> Integer,
        url -> Text,
        version -> Text,
        https -> Bool,
        https_redirect -> Bool,
        country_id -> Text,
        attachments -> Bool,
    }
}
