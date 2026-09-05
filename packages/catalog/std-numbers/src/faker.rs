pub mod address;
pub mod company;
pub mod internet;
pub mod lorem;
pub mod name;
pub mod number;
pub mod phone;

#[cfg(all(test, feature = "execute"))]
mod tests {
    use fake::{
        Fake, faker::address::en::*, faker::company::en::*, faker::internet::en::*,
        faker::lorem::en::*, faker::name::en::*, faker::phone_number::en::*,
    };
    use rand::Rng;

    // ===== Address Tests =====

    #[test]
    fn test_street_name_generates_non_empty() {
        let street: String = StreetName().fake();
        assert!(!street.is_empty());
    }

    #[test]
    fn test_street_address_format() {
        let building: String = BuildingNumber().fake();
        let street: String = StreetName().fake();
        let address = format!("{} {}", building, street);
        assert!(!address.is_empty());
        assert!(address.contains(' '));
    }

    #[test]
    fn test_city_name_generates_non_empty() {
        let city: String = CityName().fake();
        assert!(!city.is_empty());
    }

    #[test]
    fn test_state_name_generates_non_empty() {
        let state: String = StateName().fake();
        assert!(!state.is_empty());
    }

    #[test]
    fn test_country_name_generates_non_empty() {
        let country: String = CountryName().fake();
        assert!(!country.is_empty());
    }

    #[test]
    fn test_country_code_format() {
        let code: String = CountryCode().fake();
        assert!(!code.is_empty());
        assert!(code.len() >= 2);
    }

    #[test]
    fn test_post_code_generates_non_empty() {
        let code: String = PostCode().fake();
        assert!(!code.is_empty());
    }

    #[test]
    fn test_latitude_parseable() {
        let lat: String = Latitude().fake();
        let lat_f: f64 = lat.parse().expect("Latitude should be parseable as f64");
        assert!(lat_f.is_finite(), "Latitude should be a finite number");
    }

    #[test]
    fn test_longitude_parseable() {
        let lon: String = Longitude().fake();
        let lon_f: f64 = lon.parse().expect("Longitude should be parseable as f64");
        assert!(lon_f.is_finite(), "Longitude should be a finite number");
    }

    // ===== Company Tests =====

    #[test]
    fn test_company_name_generates_non_empty() {
        let company: String = CompanyName().fake();
        assert!(!company.is_empty());
    }

    #[test]
    fn test_buzzword_generates_non_empty() {
        let buzzword: String = Buzzword().fake();
        assert!(!buzzword.is_empty());
    }

    #[test]
    fn test_catch_phrase_generates_non_empty() {
        let phrase: String = CatchPhrase().fake();
        assert!(!phrase.is_empty());
    }

    #[test]
    fn test_industry_generates_non_empty() {
        let industry: String = Industry().fake();
        assert!(!industry.is_empty());
    }

    #[test]
    fn test_profession_generates_non_empty() {
        let profession: String = Profession().fake();
        assert!(!profession.is_empty());
    }

    // ===== Internet Tests =====

    #[test]
    fn test_email_format() {
        let email: String = SafeEmail().fake();
        assert!(!email.is_empty());
        assert!(email.contains('@'));
        assert!(email.contains('.'));
    }

    #[test]
    fn test_username_generates_non_empty() {
        let username: String = Username().fake();
        assert!(!username.is_empty());
    }

    #[test]
    fn test_password_length() {
        let min = 8usize;
        let max = 16usize;
        let password: String = Password(min..max).fake();
        assert!(password.len() >= min);
        assert!(password.len() < max);
    }

    #[test]
    fn test_ipv4_format() {
        let ip: String = IPv4().fake();
        let parts: Vec<&str> = ip.split('.').collect();
        assert_eq!(parts.len(), 4);
        for part in parts {
            let _num: u8 = part.parse().expect("IPv4 octet should be a valid u8");
        }
    }

    #[test]
    fn test_ipv6_format() {
        let ip: String = IPv6().fake();
        assert!(!ip.is_empty());
        let parts: Vec<&str> = ip.split(':').collect();
        assert_eq!(parts.len(), 8);
    }

    #[test]
    fn test_user_agent_generates_non_empty() {
        let ua: String = UserAgent().fake();
        assert!(!ua.is_empty());
    }

    #[test]
    fn test_domain_suffix_generates_non_empty() {
        let suffix: String = DomainSuffix().fake();
        assert!(!suffix.is_empty());
    }

    // ===== Lorem Tests =====

    #[test]
    fn test_word_generates_non_empty() {
        let word: String = Word().fake();
        assert!(!word.is_empty());
    }

    #[test]
    fn test_words_count() {
        let min = 3usize;
        let max = 6usize;
        let words: Vec<String> = Words(min..max).fake();
        assert!(words.len() >= min);
        assert!(words.len() < max);
    }

    #[test]
    fn test_sentence_ends_with_period_or_has_content() {
        let sentence: String = Sentence(4..10).fake();
        assert!(!sentence.is_empty());
    }

    #[test]
    fn test_sentences_count() {
        let min = 2usize;
        let max = 5usize;
        let sentences: Vec<String> = Sentences(min..max).fake();
        assert!(sentences.len() >= min);
        assert!(sentences.len() < max);
    }

    #[test]
    fn test_paragraph_generates_non_empty() {
        let paragraph: String = Paragraph(3..7).fake();
        assert!(!paragraph.is_empty());
    }

    #[test]
    fn test_paragraphs_count() {
        let min = 2usize;
        let max = 4usize;
        let paragraphs: Vec<String> = Paragraphs(min..max).fake();
        assert!(paragraphs.len() >= min);
        assert!(paragraphs.len() < max);
    }

    // ===== Name Tests =====

    #[test]
    fn test_first_name_generates_non_empty() {
        let name: String = FirstName().fake();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_last_name_generates_non_empty() {
        let name: String = LastName().fake();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_full_name_generates_non_empty() {
        let name: String = Name().fake();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_title_generates_non_empty() {
        let title: String = Title().fake();
        assert!(!title.is_empty());
    }

    // ===== Number Tests =====

    #[test]
    fn test_integer_range() {
        let min = 10i64;
        let max = 50i64;
        let number: i64 = (min..max).fake();
        assert!(number >= min);
        assert!(number < max);
    }

    #[test]
    fn test_float_range() {
        let min = 0.0f64;
        let max = 100.0f64;
        let mut rng = rand::rng();
        let number: f64 = rng.random_range(min..max);
        assert!(number >= min);
        assert!(number < max);
    }

    #[test]
    fn test_boolean_probability() {
        let iterations = 1000;
        let mut true_count = 0;
        let prob = 0.7f64;
        let mut rng = rand::rng();

        for _ in 0..iterations {
            if rng.random::<f64>() < prob {
                true_count += 1;
            }
        }

        let ratio = true_count as f64 / iterations as f64;
        assert!(ratio > 0.5);
        assert!(ratio < 0.9);
    }

    #[test]
    fn test_digit_range() {
        let digit: i64 = (0i64..10i64).fake();
        assert!(digit >= 0);
        assert!(digit < 10);
    }

    // ===== Phone Tests =====

    #[test]
    fn test_phone_number_generates_non_empty() {
        let phone: String = PhoneNumber().fake();
        assert!(!phone.is_empty());
    }

    #[test]
    fn test_cell_number_generates_non_empty() {
        let phone: String = CellNumber().fake();
        assert!(!phone.is_empty());
    }

    // ===== Uniqueness Tests =====

    #[test]
    fn test_faker_generates_variety() {
        let emails: Vec<String> = (0..10).map(|_| SafeEmail().fake()).collect();
        let unique: std::collections::HashSet<_> = emails.iter().collect();
        assert!(unique.len() > 1, "Expected varied fake emails");
    }

    #[test]
    fn test_faker_usernames_vary() {
        let usernames: Vec<String> = (0..10).map(|_| Username().fake()).collect();
        let unique: std::collections::HashSet<_> = usernames.iter().collect();
        assert!(unique.len() > 1, "Expected varied usernames");
    }

    #[test]
    fn test_faker_names_vary() {
        let names: Vec<String> = (0..10).map(|_| Name().fake()).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert!(unique.len() > 1, "Expected varied names");
    }
}
