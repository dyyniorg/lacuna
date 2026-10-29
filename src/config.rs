//! Municipality catalog used by the web picker and the ingest cache.
//!
//! The list is the 2026 municipality classification from Statistics Finland
//! (Tilastokeskus): https://stat.fi/fi/luokitukset/kunta/kunta_1_20260101
//!
//! Codes are the official three-digit municipality numbers. Names are in
//! Finnish.

use std::fmt::Display;

/// One municipality from the Statistics Finland classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Municipality {
    pub code: &'static str,
    pub name: &'static str,
}

impl Display for Municipality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.code)
    }
}

impl Municipality {
    /// Returns a lowercase ASCII slug for file and layer names.
    pub fn slug(&self) -> String {
        self.name
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .map(|c| match c {
                'ä' | 'å' => 'a',
                'ö' => 'o',
                _ => c,
            })
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect()
    }
}

/// All 308 municipalities in the 2026 classification.
pub const MUNICIPALITIES: &[Municipality] = &[
    Municipality {
        code: "020",
        name: "Akaa",
    },
    Municipality {
        code: "005",
        name: "Alajärvi",
    },
    Municipality {
        code: "009",
        name: "Alavieska",
    },
    Municipality {
        code: "010",
        name: "Alavus",
    },
    Municipality {
        code: "016",
        name: "Asikkala",
    },
    Municipality {
        code: "018",
        name: "Askola",
    },
    Municipality {
        code: "019",
        name: "Aura",
    },
    Municipality {
        code: "035",
        name: "Brändö",
    },
    Municipality {
        code: "043",
        name: "Eckerö",
    },
    Municipality {
        code: "046",
        name: "Enonkoski",
    },
    Municipality {
        code: "047",
        name: "Enontekiö",
    },
    Municipality {
        code: "049",
        name: "Espoo",
    },
    Municipality {
        code: "050",
        name: "Eura",
    },
    Municipality {
        code: "051",
        name: "Eurajoki",
    },
    Municipality {
        code: "052",
        name: "Evijärvi",
    },
    Municipality {
        code: "060",
        name: "Finström",
    },
    Municipality {
        code: "061",
        name: "Forssa",
    },
    Municipality {
        code: "062",
        name: "Föglö",
    },
    Municipality {
        code: "065",
        name: "Geta",
    },
    Municipality {
        code: "069",
        name: "Haapajärvi",
    },
    Municipality {
        code: "071",
        name: "Haapavesi",
    },
    Municipality {
        code: "072",
        name: "Hailuoto",
    },
    Municipality {
        code: "074",
        name: "Halsua",
    },
    Municipality {
        code: "075",
        name: "Hamina",
    },
    Municipality {
        code: "076",
        name: "Hammarland",
    },
    Municipality {
        code: "077",
        name: "Hankasalmi",
    },
    Municipality {
        code: "078",
        name: "Hanko",
    },
    Municipality {
        code: "079",
        name: "Harjavalta",
    },
    Municipality {
        code: "081",
        name: "Hartola",
    },
    Municipality {
        code: "082",
        name: "Hattula",
    },
    Municipality {
        code: "086",
        name: "Hausjärvi",
    },
    Municipality {
        code: "111",
        name: "Heinola",
    },
    Municipality {
        code: "090",
        name: "Heinävesi",
    },
    Municipality {
        code: "091",
        name: "Helsinki",
    },
    Municipality {
        code: "097",
        name: "Hirvensalmi",
    },
    Municipality {
        code: "098",
        name: "Hollola",
    },
    Municipality {
        code: "102",
        name: "Huittinen",
    },
    Municipality {
        code: "103",
        name: "Humppila",
    },
    Municipality {
        code: "105",
        name: "Hyrynsalmi",
    },
    Municipality {
        code: "106",
        name: "Hyvinkää",
    },
    Municipality {
        code: "108",
        name: "Hämeenkyrö",
    },
    Municipality {
        code: "109",
        name: "Hämeenlinna",
    },
    Municipality {
        code: "139",
        name: "Ii",
    },
    Municipality {
        code: "140",
        name: "Iisalmi",
    },
    Municipality {
        code: "142",
        name: "Iitti",
    },
    Municipality {
        code: "143",
        name: "Ikaalinen",
    },
    Municipality {
        code: "145",
        name: "Ilmajoki",
    },
    Municipality {
        code: "146",
        name: "Ilomantsi",
    },
    Municipality {
        code: "153",
        name: "Imatra",
    },
    Municipality {
        code: "148",
        name: "Inari",
    },
    Municipality {
        code: "149",
        name: "Inkoo",
    },
    Municipality {
        code: "151",
        name: "Isojoki",
    },
    Municipality {
        code: "152",
        name: "Isokyrö",
    },
    Municipality {
        code: "165",
        name: "Janakkala",
    },
    Municipality {
        code: "167",
        name: "Joensuu",
    },
    Municipality {
        code: "169",
        name: "Jokioinen",
    },
    Municipality {
        code: "170",
        name: "Jomala",
    },
    Municipality {
        code: "171",
        name: "Joroinen",
    },
    Municipality {
        code: "172",
        name: "Joutsa",
    },
    Municipality {
        code: "176",
        name: "Juuka",
    },
    Municipality {
        code: "177",
        name: "Juupajoki",
    },
    Municipality {
        code: "178",
        name: "Juva",
    },
    Municipality {
        code: "179",
        name: "Jyväskylä",
    },
    Municipality {
        code: "181",
        name: "Jämijärvi",
    },
    Municipality {
        code: "182",
        name: "Jämsä",
    },
    Municipality {
        code: "186",
        name: "Järvenpää",
    },
    Municipality {
        code: "202",
        name: "Kaarina",
    },
    Municipality {
        code: "204",
        name: "Kaavi",
    },
    Municipality {
        code: "205",
        name: "Kajaani",
    },
    Municipality {
        code: "208",
        name: "Kalajoki",
    },
    Municipality {
        code: "211",
        name: "Kangasala",
    },
    Municipality {
        code: "213",
        name: "Kangasniemi",
    },
    Municipality {
        code: "214",
        name: "Kankaanpää",
    },
    Municipality {
        code: "216",
        name: "Kannonkoski",
    },
    Municipality {
        code: "217",
        name: "Kannus",
    },
    Municipality {
        code: "218",
        name: "Karijoki",
    },
    Municipality {
        code: "224",
        name: "Karkkila",
    },
    Municipality {
        code: "226",
        name: "Karstula",
    },
    Municipality {
        code: "230",
        name: "Karvia",
    },
    Municipality {
        code: "231",
        name: "Kaskinen",
    },
    Municipality {
        code: "232",
        name: "Kauhajoki",
    },
    Municipality {
        code: "233",
        name: "Kauhava",
    },
    Municipality {
        code: "235",
        name: "Kauniainen",
    },
    Municipality {
        code: "236",
        name: "Kaustinen",
    },
    Municipality {
        code: "239",
        name: "Keitele",
    },
    Municipality {
        code: "240",
        name: "Kemi",
    },
    Municipality {
        code: "320",
        name: "Kemijärvi",
    },
    Municipality {
        code: "241",
        name: "Keminmaa",
    },
    Municipality {
        code: "322",
        name: "Kemiönsaari",
    },
    Municipality {
        code: "244",
        name: "Kempele",
    },
    Municipality {
        code: "245",
        name: "Kerava",
    },
    Municipality {
        code: "249",
        name: "Keuruu",
    },
    Municipality {
        code: "250",
        name: "Kihniö",
    },
    Municipality {
        code: "256",
        name: "Kinnula",
    },
    Municipality {
        code: "257",
        name: "Kirkkonummi",
    },
    Municipality {
        code: "260",
        name: "Kitee",
    },
    Municipality {
        code: "261",
        name: "Kittilä",
    },
    Municipality {
        code: "263",
        name: "Kiuruvesi",
    },
    Municipality {
        code: "265",
        name: "Kivijärvi",
    },
    Municipality {
        code: "271",
        name: "Kokemäki",
    },
    Municipality {
        code: "272",
        name: "Kokkola",
    },
    Municipality {
        code: "273",
        name: "Kolari",
    },
    Municipality {
        code: "275",
        name: "Konnevesi",
    },
    Municipality {
        code: "276",
        name: "Kontiolahti",
    },
    Municipality {
        code: "280",
        name: "Korsnäs",
    },
    Municipality {
        code: "284",
        name: "Koski Tl",
    },
    Municipality {
        code: "285",
        name: "Kotka",
    },
    Municipality {
        code: "286",
        name: "Kouvola",
    },
    Municipality {
        code: "287",
        name: "Kristiinankaupunki",
    },
    Municipality {
        code: "288",
        name: "Kruunupyy",
    },
    Municipality {
        code: "290",
        name: "Kuhmo",
    },
    Municipality {
        code: "291",
        name: "Kuhmoinen",
    },
    Municipality {
        code: "295",
        name: "Kumlinge",
    },
    Municipality {
        code: "297",
        name: "Kuopio",
    },
    Municipality {
        code: "300",
        name: "Kuortane",
    },
    Municipality {
        code: "301",
        name: "Kurikka",
    },
    Municipality {
        code: "304",
        name: "Kustavi",
    },
    Municipality {
        code: "305",
        name: "Kuusamo",
    },
    Municipality {
        code: "312",
        name: "Kyyjärvi",
    },
    Municipality {
        code: "316",
        name: "Kärkölä",
    },
    Municipality {
        code: "317",
        name: "Kärsämäki",
    },
    Municipality {
        code: "318",
        name: "Kökar",
    },
    Municipality {
        code: "398",
        name: "Lahti",
    },
    Municipality {
        code: "399",
        name: "Laihia",
    },
    Municipality {
        code: "400",
        name: "Laitila",
    },
    Municipality {
        code: "407",
        name: "Lapinjärvi",
    },
    Municipality {
        code: "402",
        name: "Lapinlahti",
    },
    Municipality {
        code: "403",
        name: "Lappajärvi",
    },
    Municipality {
        code: "405",
        name: "Lappeenranta",
    },
    Municipality {
        code: "408",
        name: "Lapua",
    },
    Municipality {
        code: "410",
        name: "Laukaa",
    },
    Municipality {
        code: "416",
        name: "Lemi",
    },
    Municipality {
        code: "417",
        name: "Lemland",
    },
    Municipality {
        code: "418",
        name: "Lempäälä",
    },
    Municipality {
        code: "420",
        name: "Leppävirta",
    },
    Municipality {
        code: "421",
        name: "Lestijärvi",
    },
    Municipality {
        code: "422",
        name: "Lieksa",
    },
    Municipality {
        code: "423",
        name: "Lieto",
    },
    Municipality {
        code: "425",
        name: "Liminka",
    },
    Municipality {
        code: "426",
        name: "Liperi",
    },
    Municipality {
        code: "444",
        name: "Lohja",
    },
    Municipality {
        code: "430",
        name: "Loimaa",
    },
    Municipality {
        code: "433",
        name: "Loppi",
    },
    Municipality {
        code: "434",
        name: "Loviisa",
    },
    Municipality {
        code: "435",
        name: "Luhanka",
    },
    Municipality {
        code: "436",
        name: "Lumijoki",
    },
    Municipality {
        code: "438",
        name: "Lumparland",
    },
    Municipality {
        code: "440",
        name: "Luoto",
    },
    Municipality {
        code: "441",
        name: "Luumäki",
    },
    Municipality {
        code: "475",
        name: "Maalahti",
    },
    Municipality {
        code: "478",
        name: "Maarianhamina - Mariehamn",
    },
    Municipality {
        code: "480",
        name: "Marttila",
    },
    Municipality {
        code: "481",
        name: "Masku",
    },
    Municipality {
        code: "483",
        name: "Merijärvi",
    },
    Municipality {
        code: "484",
        name: "Merikarvia",
    },
    Municipality {
        code: "489",
        name: "Miehikkälä",
    },
    Municipality {
        code: "491",
        name: "Mikkeli",
    },
    Municipality {
        code: "494",
        name: "Muhos",
    },
    Municipality {
        code: "495",
        name: "Multia",
    },
    Municipality {
        code: "498",
        name: "Muonio",
    },
    Municipality {
        code: "499",
        name: "Mustasaari",
    },
    Municipality {
        code: "500",
        name: "Muurame",
    },
    Municipality {
        code: "503",
        name: "Mynämäki",
    },
    Municipality {
        code: "504",
        name: "Myrskylä",
    },
    Municipality {
        code: "505",
        name: "Mäntsälä",
    },
    Municipality {
        code: "508",
        name: "Mänttä-Vilppula",
    },
    Municipality {
        code: "507",
        name: "Mäntyharju",
    },
    Municipality {
        code: "529",
        name: "Naantali",
    },
    Municipality {
        code: "531",
        name: "Nakkila",
    },
    Municipality {
        code: "535",
        name: "Nivala",
    },
    Municipality {
        code: "536",
        name: "Nokia",
    },
    Municipality {
        code: "538",
        name: "Nousiainen",
    },
    Municipality {
        code: "541",
        name: "Nurmes",
    },
    Municipality {
        code: "543",
        name: "Nurmijärvi",
    },
    Municipality {
        code: "545",
        name: "Närpiö",
    },
    Municipality {
        code: "560",
        name: "Orimattila",
    },
    Municipality {
        code: "561",
        name: "Oripää",
    },
    Municipality {
        code: "562",
        name: "Orivesi",
    },
    Municipality {
        code: "563",
        name: "Oulainen",
    },
    Municipality {
        code: "564",
        name: "Oulu",
    },
    Municipality {
        code: "309",
        name: "Outokumpu",
    },
    Municipality {
        code: "576",
        name: "Padasjoki",
    },
    Municipality {
        code: "577",
        name: "Paimio",
    },
    Municipality {
        code: "578",
        name: "Paltamo",
    },
    Municipality {
        code: "445",
        name: "Parainen",
    },
    Municipality {
        code: "580",
        name: "Parikkala",
    },
    Municipality {
        code: "581",
        name: "Parkano",
    },
    Municipality {
        code: "599",
        name: "Pedersören kunta",
    },
    Municipality {
        code: "583",
        name: "Pelkosenniemi",
    },
    Municipality {
        code: "854",
        name: "Pello",
    },
    Municipality {
        code: "584",
        name: "Perho",
    },
    Municipality {
        code: "592",
        name: "Petäjävesi",
    },
    Municipality {
        code: "593",
        name: "Pieksämäki",
    },
    Municipality {
        code: "595",
        name: "Pielavesi",
    },
    Municipality {
        code: "598",
        name: "Pietarsaari",
    },
    Municipality {
        code: "601",
        name: "Pihtipudas",
    },
    Municipality {
        code: "604",
        name: "Pirkkala",
    },
    Municipality {
        code: "607",
        name: "Polvijärvi",
    },
    Municipality {
        code: "608",
        name: "Pomarkku",
    },
    Municipality {
        code: "609",
        name: "Pori",
    },
    Municipality {
        code: "611",
        name: "Pornainen",
    },
    Municipality {
        code: "638",
        name: "Porvoo",
    },
    Municipality {
        code: "614",
        name: "Posio",
    },
    Municipality {
        code: "615",
        name: "Pudasjärvi",
    },
    Municipality {
        code: "616",
        name: "Pukkila",
    },
    Municipality {
        code: "619",
        name: "Punkalaidun",
    },
    Municipality {
        code: "620",
        name: "Puolanka",
    },
    Municipality {
        code: "623",
        name: "Puumala",
    },
    Municipality {
        code: "624",
        name: "Pyhtää",
    },
    Municipality {
        code: "625",
        name: "Pyhäjoki",
    },
    Municipality {
        code: "626",
        name: "Pyhäjärvi",
    },
    Municipality {
        code: "630",
        name: "Pyhäntä",
    },
    Municipality {
        code: "631",
        name: "Pyhäranta",
    },
    Municipality {
        code: "635",
        name: "Pälkäne",
    },
    Municipality {
        code: "636",
        name: "Pöytyä",
    },
    Municipality {
        code: "678",
        name: "Raahe",
    },
    Municipality {
        code: "710",
        name: "Raasepori",
    },
    Municipality {
        code: "680",
        name: "Raisio",
    },
    Municipality {
        code: "681",
        name: "Rantasalmi",
    },
    Municipality {
        code: "683",
        name: "Ranua",
    },
    Municipality {
        code: "684",
        name: "Rauma",
    },
    Municipality {
        code: "686",
        name: "Rautalampi",
    },
    Municipality {
        code: "687",
        name: "Rautavaara",
    },
    Municipality {
        code: "689",
        name: "Rautjärvi",
    },
    Municipality {
        code: "691",
        name: "Reisjärvi",
    },
    Municipality {
        code: "694",
        name: "Riihimäki",
    },
    Municipality {
        code: "697",
        name: "Ristijärvi",
    },
    Municipality {
        code: "698",
        name: "Rovaniemi",
    },
    Municipality {
        code: "700",
        name: "Ruokolahti",
    },
    Municipality {
        code: "702",
        name: "Ruovesi",
    },
    Municipality {
        code: "704",
        name: "Rusko",
    },
    Municipality {
        code: "707",
        name: "Rääkkylä",
    },
    Municipality {
        code: "729",
        name: "Saarijärvi",
    },
    Municipality {
        code: "732",
        name: "Salla",
    },
    Municipality {
        code: "734",
        name: "Salo",
    },
    Municipality {
        code: "736",
        name: "Saltvik",
    },
    Municipality {
        code: "790",
        name: "Sastamala",
    },
    Municipality {
        code: "738",
        name: "Sauvo",
    },
    Municipality {
        code: "739",
        name: "Savitaipale",
    },
    Municipality {
        code: "740",
        name: "Savonlinna",
    },
    Municipality {
        code: "742",
        name: "Savukoski",
    },
    Municipality {
        code: "743",
        name: "Seinäjoki",
    },
    Municipality {
        code: "746",
        name: "Sievi",
    },
    Municipality {
        code: "747",
        name: "Siikainen",
    },
    Municipality {
        code: "748",
        name: "Siikajoki",
    },
    Municipality {
        code: "791",
        name: "Siikalatva",
    },
    Municipality {
        code: "749",
        name: "Siilinjärvi",
    },
    Municipality {
        code: "751",
        name: "Simo",
    },
    Municipality {
        code: "753",
        name: "Sipoo",
    },
    Municipality {
        code: "755",
        name: "Siuntio",
    },
    Municipality {
        code: "758",
        name: "Sodankylä",
    },
    Municipality {
        code: "759",
        name: "Soini",
    },
    Municipality {
        code: "761",
        name: "Somero",
    },
    Municipality {
        code: "762",
        name: "Sonkajärvi",
    },
    Municipality {
        code: "765",
        name: "Sotkamo",
    },
    Municipality {
        code: "766",
        name: "Sottunga",
    },
    Municipality {
        code: "768",
        name: "Sulkava",
    },
    Municipality {
        code: "771",
        name: "Sund",
    },
    Municipality {
        code: "777",
        name: "Suomussalmi",
    },
    Municipality {
        code: "778",
        name: "Suonenjoki",
    },
    Municipality {
        code: "781",
        name: "Sysmä",
    },
    Municipality {
        code: "783",
        name: "Säkylä",
    },
    Municipality {
        code: "831",
        name: "Taipalsaari",
    },
    Municipality {
        code: "832",
        name: "Taivalkoski",
    },
    Municipality {
        code: "833",
        name: "Taivassalo",
    },
    Municipality {
        code: "834",
        name: "Tammela",
    },
    Municipality {
        code: "837",
        name: "Tampere",
    },
    Municipality {
        code: "844",
        name: "Tervo",
    },
    Municipality {
        code: "845",
        name: "Tervola",
    },
    Municipality {
        code: "846",
        name: "Teuva",
    },
    Municipality {
        code: "848",
        name: "Tohmajärvi",
    },
    Municipality {
        code: "849",
        name: "Toholampi",
    },
    Municipality {
        code: "850",
        name: "Toivakka",
    },
    Municipality {
        code: "851",
        name: "Tornio",
    },
    Municipality {
        code: "853",
        name: "Turku",
    },
    Municipality {
        code: "857",
        name: "Tuusniemi",
    },
    Municipality {
        code: "858",
        name: "Tuusula",
    },
    Municipality {
        code: "859",
        name: "Tyrnävä",
    },
    Municipality {
        code: "886",
        name: "Ulvila",
    },
    Municipality {
        code: "887",
        name: "Urjala",
    },
    Municipality {
        code: "889",
        name: "Utajärvi",
    },
    Municipality {
        code: "890",
        name: "Utsjoki",
    },
    Municipality {
        code: "892",
        name: "Uurainen",
    },
    Municipality {
        code: "893",
        name: "Uusikaarlepyy",
    },
    Municipality {
        code: "895",
        name: "Uusikaupunki",
    },
    Municipality {
        code: "785",
        name: "Vaala",
    },
    Municipality {
        code: "905",
        name: "Vaasa",
    },
    Municipality {
        code: "908",
        name: "Valkeakoski",
    },
    Municipality {
        code: "092",
        name: "Vantaa",
    },
    Municipality {
        code: "915",
        name: "Varkaus",
    },
    Municipality {
        code: "918",
        name: "Vehmaa",
    },
    Municipality {
        code: "921",
        name: "Vesanto",
    },
    Municipality {
        code: "922",
        name: "Vesilahti",
    },
    Municipality {
        code: "924",
        name: "Veteli",
    },
    Municipality {
        code: "925",
        name: "Vieremä",
    },
    Municipality {
        code: "927",
        name: "Vihti",
    },
    Municipality {
        code: "931",
        name: "Viitasaari",
    },
    Municipality {
        code: "934",
        name: "Vimpeli",
    },
    Municipality {
        code: "935",
        name: "Virolahti",
    },
    Municipality {
        code: "936",
        name: "Virrat",
    },
    Municipality {
        code: "941",
        name: "Vårdö",
    },
    Municipality {
        code: "946",
        name: "Vöyri",
    },
    Municipality {
        code: "976",
        name: "Ylitornio",
    },
    Municipality {
        code: "977",
        name: "Ylivieska",
    },
    Municipality {
        code: "980",
        name: "Ylöjärvi",
    },
    Municipality {
        code: "981",
        name: "Ypäjä",
    },
    Municipality {
        code: "989",
        name: "Ähtäri",
    },
    Municipality {
        code: "992",
        name: "Äänekoski",
    },
];

/// Looks up a municipality by its official code.
pub fn municipality(code: &str) -> Option<&'static Municipality> {
    MUNICIPALITIES.iter().find(|m| m.code == code)
}
