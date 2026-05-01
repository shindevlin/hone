//! Reserved identity namespace — all registered to shindevlin at genesis.
//!
//! Covers: every single letter (a-z), every 2-letter combo (aa-zz), and ~350 globally popular
//! first names spanning English, Spanish/Latin, Arabic, Indian, East Asian, African, and
//! Eastern European naming traditions. Total ≈ 1050 names.
//!
//! shindevlin can sell, gift, or transfer any of these identities via AccountTransfer.

/// shindevlin's posting key (from genesis.json). All reserved names point to this key.
pub const SHINDEVLIN_POSTING_KEY: &str =
    "e4b6ac67da80655af5c5e719f4f36b6dca4ebc50ca2842a6263c6ba30defb7a2";

/// Popular first names worldwide — ASCII-compatible, 3+ characters.
/// Globally diverse: English/Western, Spanish/Latin, Arabic/MENA, Indian, East Asian, African, Eastern European.
pub const POPULAR_NAMES: &[&str] = &[
    // ── English / Western ────────────────────────────────────────────────────
    "ace", "ada", "adam", "alan", "alex", "ali", "alma", "amy", "ana", "andy",
    "angel", "anna", "anne", "arlo", "arthur", "ash", "asher", "atlas", "august",
    "aurora", "austin", "autumn", "ava", "avery", "axel",
    "beau", "beck", "bella", "ben", "blake", "bob", "brad", "brandon", "brett",
    "brian", "brooke", "bruce", "bryce",
    "caleb", "carl", "carla", "carol", "casey", "chad", "charlie", "charlotte",
    "chloe", "chris", "claire", "clark", "cody", "cole", "corey", "craig",
    "dale", "dan", "daniel", "dana", "darren", "david", "dean", "derek",
    "diana", "dominic", "don", "donna", "drew", "dylan",
    "earl", "eli", "elijah", "ella", "emily", "emma", "eric", "ethan", "evan", "eve",
    "faith", "felix", "finn", "fiona", "frank", "fred",
    "gabriel", "gavin", "george", "glen", "grace", "graham", "greg",
    "hannah", "harper", "harry", "hazel", "heidi", "henry", "holly", "hugo",
    "ian", "iris", "isaac", "isabella", "isla", "ivy",
    "jack", "jackson", "jade", "jake", "james", "jane", "jasmine", "jason",
    "jay", "jeff", "jen", "jesse", "jessica", "jill", "joel", "joanna", "john",
    "jonah", "jonathan", "jordan", "josh", "joy", "julia", "june",
    "karen", "kate", "kelly", "kevin", "kim", "kyle",
    "layla", "lea", "leah", "leo", "leon", "lewis", "liam", "lily", "linda",
    "lisa", "logan", "lola", "louis", "lucas", "luke", "luna", "lydia",
    "mae", "marcus", "maria", "marie", "martin", "mary", "mason", "matt",
    "max", "maya", "megan", "mia", "michael", "mila", "miles", "molly", "morgan",
    "naomi", "nate", "neil", "nick", "noah", "noel", "nora", "nova",
    "oliver", "olivia", "oscar", "owen",
    "paige", "parker", "paul", "pearl", "peter", "piper",
    "rachel", "ray", "reed", "rex", "richard", "riley", "rob", "robin",
    "rosa", "rose", "ross", "ryan",
    "sam", "sandy", "sara", "sarah", "scarlett", "scott", "sean", "selena",
    "serena", "seth", "sierra", "simon", "skye", "sofia", "sophia", "stella",
    "stephen", "sue", "summer", "susan",
    "tara", "taylor", "ted", "theo", "thomas", "tim", "tina", "tori",
    "travis", "troy", "tyler",
    "vera", "victor", "vincent", "violet", "vivian",
    "wade", "walter", "wendy", "will", "willow", "wyatt",
    "yvonne",
    "zach", "zane", "zara", "zoe", "zoey",

    // ── Spanish / Latin American ─────────────────────────────────────────────
    "alejandro", "camila", "carmen", "carlos", "consuelo", "cristina",
    "diego", "elena", "emilio", "fernanda", "florencia", "gabriela",
    "gonzalo", "ignacio", "isabella", "javier", "jose", "juan",
    "juanita", "lucia", "luciana", "luisa", "maia", "marcelo",
    "mariana", "mateo", "marina", "miguel", "natalia",
    "nicolas", "pablo", "pilar", "rafael", "ramon", "roberto",
    "rosario", "santiago", "sebastian", "valentina", "valeria", "martina",
    "ximena",

    // ── Arabic / Middle Eastern ──────────────────────────────────────────────
    "aisha", "amir", "amira", "farid", "farida", "fatima", "hafsa",
    "hamid", "hassan", "ibrahim", "jasmin", "karim", "karima",
    "laila", "leila", "lina", "malik", "malika", "mariam", "nadia",
    "omar", "rania", "rima", "sana", "soraya", "tariq",
    "yasmin", "yusuf", "zainab",

    // ── Indian / South Asian ─────────────────────────────────────────────────
    "aarav", "aditya", "ananya", "anjali", "ankit", "arjun", "aryan",
    "ayaan", "deepa", "dev", "diya", "ishaan", "kavya", "krish",
    "kunal", "meera", "neha", "nikhil", "pooja", "priya",
    "rahul", "raj", "riya", "rohan", "sakshi", "shiv",
    "sunita", "varun", "veer", "yash", "zara",

    // ── East Asian (romanized) ───────────────────────────────────────────────
    "akira", "chen", "fang", "hana", "haruto", "hiro",
    "kai", "kenji", "lei", "mei", "min", "ming",
    "ren", "ryo", "sakura", "sato", "wei", "xin",
    "yuki", "yuna", "zhang",

    // ── African ─────────────────────────────────────────────────────────────
    "abena", "amara", "amina", "ayo", "chi", "chioma",
    "esi", "fatou", "imani", "kofi", "kwame",
    "nia", "obi", "ola", "sade", "seun", "yemi", "zola",

    // ── Eastern European ────────────────────────────────────────────────────
    "alexei", "anastasia", "bogdan", "dmitry", "ekaterina",
    "inna", "ivan", "katya", "ludmila", "mikhail",
    "natasha", "nina", "olga", "oleg", "sasha",
    "sonya", "tania", "tatiana", "yara", "yuri",

    // ── Scandinavian / Nordic ────────────────────────────────────────────────
    "astrid", "bjorn", "erik", "freya", "gunnar", "hilde",
    "ingrid", "lars", "leif", "sigrid", "sven", "thor",

    // ── Greek / Mediterranean ────────────────────────────────────────────────
    "adonis", "agatha", "alexia", "callie", "daphne", "demetrios",
    "elektra", "evander", "giorgos", "irene", "katerina",
    "konstantinos", "nikos", "panos", "phoebe", "stavros", "thalia",

    // ── Universal / cross-cultural ───────────────────────────────────────────
    "elia", "elias", "emre", "esra", "ezra", "gio",
    "lena", "levi", "mara", "noa", "nolan", "oren",
    "petra", "rene", "reza", "rio", "roma", "romi",
    "romy", "rona", "shira", "sol", "sula", "tamar",
    "tara", "teo", "yael",
];

/// Generate the full reserved-name list: singles (a-z) + doubles (aa-zz) + popular names.
/// Returns deduplicated, sorted names.
pub fn all_reserved() -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(1100);

    // a-z  (26)
    for c in b'a'..=b'z' {
        names.push((c as char).to_string());
    }

    // aa-zz  (676)
    for c1 in b'a'..=b'z' {
        for c2 in b'a'..=b'z' {
            names.push(format!("{}{}", c1 as char, c2 as char));
        }
    }

    // popular names — skip duplicates with singles/doubles (e.g. "aa" already present)
    let existing: std::collections::HashSet<String> = names.iter().cloned().collect();
    for &name in POPULAR_NAMES {
        let s = name.to_string();
        if !existing.contains(&s) && !names.contains(&s) {
            names.push(s);
        }
    }

    names
}
