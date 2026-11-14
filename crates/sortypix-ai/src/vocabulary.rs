//! The words Sortypix AI can tag a photo with, and how each is described to the text model.
//!
//! Written for Sortypix (MIT, like the rest of the repository): subjects, places, moments,
//! framing, styles and a few booru-like words for people coming from image generation. English,
//! lower case, spaces rather than underscores ("long hair", as in a Stable Diffusion prompt).
//! One word per idea (no synonyms), no hierarchy. Left out on purpose: adult words and ethnicity.
//! Gender words (man, woman, 1girl, 1boy) are in as well as the neutral ones: Mickael's choice
//! of 2026-11-13, for people used to booru tags; they are guesses like every tag.
//!
//! Changing anything here changes [`vocabulary_hash`]: the embedded text embeddings
//! (`data/tags-l14-int8.bin`) are then refused until `ai_tags --write` regenerates them.

/// How a word is described to the text model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompts {
    /// A noun phrase put in [`THING_TEMPLATE`] ("a dog", "a group of people").
    Thing(&'static str),
    /// A whole sentence written for the word (styles, moments, framing).
    Own(&'static str),
}

/// A word of the vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    pub name: &'static str,
    pub group: Group,
    pub prompts: Prompts,
}

/// What a word is about (used to order the completions and in the reports).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    People,
    Animal,
    Vehicle,
    Food,
    Object,
    Place,
    Nature,
    Moment,
    Framing,
    Style,
}

/// The sentence a [`Prompts::Thing`] phrase is put in. One sentence per word: the mean of four
/// templates ("a photo of", "a picture of", "an image of", "an illustration of") and of several
/// sentences per word measured worse (`ai_tags`, 2026-11-13: 262 right pairs at a precision of
/// 0.796 against 268 at 0.825 at the same selection).
pub const THING_TEMPLATE: &str = "a photo of {}.";

const fn t(name: &'static str, group: Group, phrase: &'static str) -> Word {
    Word {
        name,
        group,
        prompts: Prompts::Thing(phrase),
    }
}

const fn o(name: &'static str, group: Group, sentence: &'static str) -> Word {
    Word {
        name,
        group,
        prompts: Prompts::Own(sentence),
    }
}

use Group::*;

/// The vocabulary, in a stable order (the index space of the embedded embeddings).
pub const VOCABULARY: &[Word] = &[
    // People.
    t("person", People, "a person"),
    t("people", People, "a group of people"),
    t("crowd", People, "a crowd of people"),
    t("man", People, "a man"),
    t("woman", People, "a woman"),
    o("1girl", People, "an image of one girl alone."),
    o("1boy", People, "an image of one boy alone."),
    t("child", People, "a child"),
    t("baby", People, "a baby"),
    o("portrait", People, "a portrait photo of a person."),
    o("selfie", People, "a selfie taken with a phone."),
    o("solo", People, "a photo of one single person alone."),
    o("couple", People, "a photo of a couple together."),
    o("family", People, "a family photo."),
    o("smile", People, "a smiling person."),
    o(
        "looking at viewer",
        People,
        "a person looking at the camera.",
    ),
    o("from behind", People, "a person seen from behind."),
    o(
        "full body",
        People,
        "a full body shot of a person standing.",
    ),
    o("upper body", People, "an upper body shot of a person."),
    o("sitting", People, "a person sitting."),
    o("lying down", People, "a person lying down."),
    o("long hair", People, "a person with long hair."),
    o("short hair", People, "a person with short hair."),
    o("blonde hair", People, "a person with blonde hair."),
    o("glasses", People, "a person wearing glasses."),
    o("sunglasses", People, "a person wearing sunglasses."),
    o("hat", People, "a person wearing a hat."),
    o("beard", People, "a person with a beard."),
    o("tattoo", People, "a person with tattoos."),
    o("dress", People, "a person wearing a dress."),
    o("suit", People, "a person wearing a suit and tie."),
    o("uniform", People, "a person in uniform."),
    o("costume", People, "a person in costume."),
    o("armor", People, "a person wearing armor."),
    o("dancing", People, "people dancing."),
    o("running", People, "a person running."),
    o("swimming", People, "a person swimming."),
    o("hands", People, "a close-up of hands."),
    o("face", People, "a close-up of a face."),
    // Added 2026-11-13 with the tag head (Open Images classes).
    t("athlete", People, "an athlete"),
    t("musician", People, "a musician"),
    t("soldier", People, "a soldier"),
    t("audience", People, "an audience"),
    t("brown hair", People, "a person with brown hair"),
    t("black hair", People, "a person with black hair"),
    t("jeans", People, "a person wearing jeans"),
    t("t-shirt", People, "a person wearing a t-shirt"),
    t("jacket", People, "a person wearing a jacket"),
    t("coat", People, "a person wearing a coat"),
    t("shorts", People, "a person wearing shorts"),
    t("swimwear", People, "a person in swimwear"),
    t("high heels", People, "high heels"),
    t("boots", People, "boots"),
    t("gown", People, "a woman in a gown"),
    t("helmet", People, "a person wearing a helmet"),
    t("cap", People, "a person wearing a cap"),
    t("tie", People, "a man wearing a tie"),
    t("gloves", People, "a person wearing gloves"),
    t("goggles", People, "a person wearing goggles"),
    t("walking", People, "people walking"),
    t("eyes", People, "a close view of human eyes"),
    // Animals.
    t("animal", Animal, "an animal"),
    t("cat", Animal, "a cat"),
    t("dog", Animal, "a dog"),
    t("bird", Animal, "a bird"),
    t("horse", Animal, "a horse"),
    t("cow", Animal, "a cow"),
    t("sheep", Animal, "a sheep"),
    t("goat", Animal, "a goat"),
    t("pig", Animal, "a pig"),
    t("chicken", Animal, "a chicken"),
    t("duck", Animal, "a duck"),
    t("swan", Animal, "a swan"),
    t("pigeon", Animal, "a pigeon"),
    t("seagull", Animal, "a seagull"),
    t("owl", Animal, "an owl"),
    t("parrot", Animal, "a parrot"),
    t("fish", Animal, "a fish"),
    t("insect", Animal, "an insect"),
    t("butterfly", Animal, "a butterfly"),
    t("bee", Animal, "a bee"),
    t("spider", Animal, "a spider"),
    t("squirrel", Animal, "a squirrel"),
    t("rabbit", Animal, "a rabbit"),
    t("deer", Animal, "a deer"),
    t("fox", Animal, "a fox"),
    t("bear", Animal, "a bear"),
    t("elephant", Animal, "an elephant"),
    t("giraffe", Animal, "a giraffe"),
    t("zebra", Animal, "a zebra"),
    t("lion", Animal, "a lion"),
    t("tiger", Animal, "a tiger"),
    t("monkey", Animal, "a monkey"),
    t("camel", Animal, "a camel"),
    t("turtle", Animal, "a turtle"),
    t("frog", Animal, "a frog"),
    t("snake", Animal, "a snake"),
    t("lizard", Animal, "a lizard"),
    t("dolphin", Animal, "a dolphin"),
    t("whale", Animal, "a whale"),
    t("dragon", Animal, "a dragon"),
    // Added 2026-11-13 with the tag head (Open Images classes).
    t("puppy", Animal, "a puppy"),
    t("kitten", Animal, "a kitten"),
    t("bird of prey", Animal, "a bird of prey"),
    t("antelope", Animal, "an antelope"),
    t("herd", Animal, "a herd of animals"),
    t("pet", Animal, "a pet"),
    t("wildlife", Animal, "wild animals in nature"),
    t("coral", Animal, "coral"),
    t("jellyfish", Animal, "a jellyfish"),
    t("shark", Animal, "a shark"),
    t("crab", Animal, "a crab"),
    t("eagle", Animal, "an eagle"),
    t("penguin", Animal, "a penguin"),
    t("goose", Animal, "a goose"),
    t("rodent", Animal, "a rodent"),
    t("mouse", Animal, "a mouse"),
    // Vehicles.
    t("car", Vehicle, "a car"),
    t("bus", Vehicle, "a bus"),
    t("truck", Vehicle, "a truck"),
    t("motorcycle", Vehicle, "a motorcycle"),
    t("bicycle", Vehicle, "a bicycle"),
    t("train", Vehicle, "a train"),
    t("tram", Vehicle, "a tram"),
    t("airplane", Vehicle, "an airplane"),
    t("helicopter", Vehicle, "a helicopter"),
    t("boat", Vehicle, "a boat"),
    t("sailboat", Vehicle, "a sailboat"),
    t("ship", Vehicle, "a large ship"),
    t("tractor", Vehicle, "a tractor"),
    t("spaceship", Vehicle, "a spaceship"),
    // Added 2026-11-13 with the tag head (Open Images classes).
    t("sports car", Vehicle, "a sports car"),
    t("vintage car", Vehicle, "a vintage car"),
    t("race car", Vehicle, "a race car"),
    t("van", Vehicle, "a van"),
    t("pickup truck", Vehicle, "a pickup truck"),
    t("suv", Vehicle, "an suv"),
    t("airliner", Vehicle, "an airliner"),
    t("fighter jet", Vehicle, "a fighter jet"),
    t("canoe", Vehicle, "a canoe"),
    t("skateboard", Vehicle, "a skateboard"),
    t("scooter", Vehicle, "a scooter"),
    t("cart", Vehicle, "a cart"),
    // Food and drink.
    t("food", Food, "a plate of food"),
    t("pizza", Food, "a pizza"),
    t("burger", Food, "a burger"),
    t("sandwich", Food, "a sandwich"),
    t("pasta", Food, "a plate of pasta"),
    t("sushi", Food, "sushi"),
    t("salad", Food, "a salad"),
    t("soup", Food, "a bowl of soup"),
    t("bread", Food, "bread"),
    t("cake", Food, "a cake"),
    t("dessert", Food, "a dessert"),
    t("ice cream", Food, "an ice cream"),
    t("chocolate", Food, "chocolate"),
    t("fruit", Food, "fruit"),
    t("vegetables", Food, "vegetables"),
    t("meat", Food, "a piece of meat"),
    t("cheese", Food, "cheese"),
    t("coffee", Food, "a cup of coffee"),
    t("tea", Food, "a cup of tea"),
    t("wine", Food, "a glass of wine"),
    t("beer", Food, "a glass of beer"),
    t("cocktail", Food, "a cocktail"),
    // Added 2026-11-13 with the tag head (Open Images classes).
    t("breakfast", Food, "breakfast"),
    t("fast food", Food, "fast food"),
    t("seafood", Food, "seafood"),
    t("steak", Food, "a steak"),
    t("noodles", Food, "noodles"),
    t("rice", Food, "rice"),
    t("curry", Food, "curry"),
    t("egg", Food, "eggs"),
    t("cookie", Food, "cookies"),
    t("pastry", Food, "a pastry"),
    t("candy", Food, "candy"),
    t("strawberry", Food, "strawberries"),
    t("orange", Food, "an orange fruit"),
    t("tomato", Food, "tomatoes"),
    t("mushroom", Food, "mushrooms"),
    t("juice", Food, "a glass of juice"),
    t("soft drink", Food, "a soft drink"),
    t("birthday cake", Food, "a birthday cake"),
    t("cupcake", Food, "a cupcake"),
    t("barbecue", Food, "a barbecue"),
    // Objects.
    t("book", Object, "a book"),
    t("phone", Object, "a smartphone"),
    t("computer", Object, "a computer"),
    t("camera", Object, "a camera"),
    t("guitar", Object, "a guitar"),
    t("piano", Object, "a piano"),
    t("drums", Object, "a drum kit"),
    t("microphone", Object, "a microphone"),
    t("clock", Object, "a clock"),
    t("chair", Object, "a chair"),
    t("table", Object, "a table"),
    t("bed", Object, "a bed"),
    t("sofa", Object, "a sofa"),
    t("lamp", Object, "a lamp"),
    t("candle", Object, "a lit candle"),
    t("vase", Object, "a vase"),
    t("bottle", Object, "a bottle"),
    t("umbrella", Object, "an umbrella"),
    t("bag", Object, "a bag"),
    t("shoes", Object, "a pair of shoes"),
    t("toy", Object, "a toy"),
    t("teddy bear", Object, "a teddy bear"),
    t("balloon", Object, "balloons"),
    t("flag", Object, "a flag"),
    t("sign", Object, "a sign with writing on it"),
    t("statue", Object, "a statue"),
    t("fountain", Object, "a fountain"),
    t("bench", Object, "a bench"),
    t("ball", Object, "a ball"),
    t("gift", Object, "a wrapped gift"),
    t("christmas tree", Object, "a christmas tree"),
    t("sword", Object, "a sword"),
    t("weapon", Object, "a weapon"),
    t("jewelry", Object, "jewelry"),
    t("mirror", Object, "a mirror"),
    t("window", Object, "a window"),
    t("door", Object, "a door"),
    t("stairs", Object, "stairs"),
    // Added 2026-11-13 with the tag head (Open Images classes).
    t("laptop", Object, "a laptop"),
    t("monitor", Object, "a computer monitor"),
    t("keyboard", Object, "a computer keyboard"),
    t("doll", Object, "a doll"),
    t("box", Object, "a box"),
    t("basket", Object, "a basket"),
    t("cup", Object, "a cup"),
    t("plate", Object, "a plate"),
    t("bowl", Object, "a bowl"),
    t("spoon", Object, "a spoon"),
    t("fork", Object, "a fork"),
    t("knife", Object, "a knife"),
    t("wine glass", Object, "a wine glass"),
    t("jar", Object, "a jar"),
    t("pillow", Object, "a pillow"),
    t("carpet", Object, "a carpet"),
    t("curtain", Object, "a curtain"),
    t("shelf", Object, "a shelf"),
    t("bookcase", Object, "a bookcase"),
    t("desk", Object, "a desk"),
    t("cupboard", Object, "a cupboard"),
    t("poster", Object, "a poster"),
    t("banner", Object, "a banner"),
    t("gun", Object, "a gun"),
    t("rope", Object, "a rope"),
    t("tent", Object, "a tent"),
    t("mask", Object, "a mask"),
    t("headphones", Object, "headphones"),
    t("watch", Object, "a wristwatch"),
    t("backpack", Object, "a backpack"),
    t("suitcase", Object, "a suitcase"),
    t("sculpture", Object, "a sculpture"),
    t("text", Object, "text"),
    t("paper", Object, "paper"),
    t("television", Object, "a television"),
    t("lantern", Object, "a lantern"),
    t("fireplace", Object, "a fireplace"),
    t("kite", Object, "a kite"),
    t("surfboard", Object, "a surfboard"),
    t("skis", Object, "skis"),
    // Places.
    t("city", Place, "a city"),
    t("street", Place, "a street"),
    t("building", Place, "a building"),
    t("skyscraper", Place, "skyscrapers"),
    t("house", Place, "a house"),
    t("church", Place, "a church"),
    t("castle", Place, "a castle"),
    t("temple", Place, "a temple"),
    t("mosque", Place, "a mosque"),
    t("ruins", Place, "ancient ruins"),
    t("tower", Place, "a tower"),
    t("lighthouse", Place, "a lighthouse"),
    t("windmill", Place, "a windmill"),
    t("bridge", Place, "a bridge"),
    t("road", Place, "a road"),
    t("harbor", Place, "a harbor with boats"),
    t("market", Place, "a market"),
    t("shop", Place, "a shop"),
    t("restaurant", Place, "a restaurant"),
    t("bar", Place, "a bar"),
    t("kitchen", Place, "a kitchen"),
    t("living room", Place, "a living room"),
    t("bedroom", Place, "a bedroom"),
    t("bathroom", Place, "a bathroom"),
    t("office", Place, "an office"),
    t("classroom", Place, "a classroom"),
    t("library", Place, "a library"),
    t("museum", Place, "a museum"),
    t("stadium", Place, "a stadium"),
    t("stage", Place, "a stage with performers"),
    t("airport", Place, "an airport"),
    t("train station", Place, "a train station"),
    t("playground", Place, "a playground"),
    t("swimming pool", Place, "a swimming pool"),
    t("farm", Place, "a farm"),
    t("village", Place, "a village"),
    t("garden", Place, "a garden"),
    t("park", Place, "a park"),
    t("graffiti", Place, "graffiti on a wall"),
    o("indoors", Place, "a photo taken indoors."),
    o("outdoors", Place, "a photo taken outdoors."),
    // Added 2026-11-13 with the tag head (Open Images classes).
    t("cottage", Place, "a cottage"),
    t("hut", Place, "a hut"),
    t("shed", Place, "a shed"),
    t("barn", Place, "a barn"),
    t("monument", Place, "a monument"),
    t("arch", Place, "an arch"),
    t("columns", Place, "columns"),
    t("walkway", Place, "a walkway"),
    t("highway", Place, "a highway"),
    t("race track", Place, "a race track"),
    t("runway", Place, "an airport runway"),
    t("zoo", Place, "a zoo"),
    t("countryside", Place, "the countryside"),
    t("neighborhood", Place, "a residential neighborhood"),
    t("backyard", Place, "a backyard"),
    t("lawn", Place, "a lawn"),
    t("roof", Place, "a roof"),
    t("facade", Place, "a building facade"),
    t("wall", Place, "a wall"),
    t("floor", Place, "a floor"),
    t("ceiling", Place, "a ceiling"),
    t("room", Place, "a room"),
    t("fence", Place, "a fence"),
    t("exhibition", Place, "an exhibition"),
    // Nature.
    t("beach", Nature, "a beach"),
    t("sea", Nature, "the sea"),
    t("lake", Nature, "a lake"),
    t("river", Nature, "a river"),
    t("waterfall", Nature, "a waterfall"),
    t("mountain", Nature, "mountains"),
    t("hill", Nature, "hills"),
    t("forest", Nature, "a forest"),
    t("tree", Nature, "a tree"),
    t("flower", Nature, "a flower"),
    t("plant", Nature, "a plant"),
    t("grass", Nature, "grass"),
    t("field", Nature, "a field"),
    t("desert", Nature, "a desert"),
    t("cave", Nature, "a cave"),
    t("island", Nature, "an island"),
    t("rocks", Nature, "rocks"),
    t("sky", Nature, "the sky"),
    t("clouds", Nature, "clouds in the sky"),
    t("rainbow", Nature, "a rainbow"),
    t("moon", Nature, "the moon"),
    o("stars", Nature, "stars in the night sky."),
    t("water", Nature, "water"),
    t("snow", Nature, "snow"),
    t("ice", Nature, "ice"),
    t("autumn leaves", Nature, "autumn leaves"),
    // Added 2026-11-13 with the tag head (Open Images classes).
    t("ocean", Nature, "the ocean"),
    t("coast", Nature, "a coast"),
    t("waves", Nature, "waves"),
    t("sand", Nature, "sand"),
    t("cliff", Nature, "a cliff"),
    t("valley", Nature, "a valley"),
    t("meadow", Nature, "a meadow"),
    t("jungle", Nature, "a jungle"),
    t("wetland", Nature, "a wetland"),
    t("pond", Nature, "a pond"),
    t("stream", Nature, "a stream"),
    t("savanna", Nature, "a savanna"),
    t("canyon", Nature, "a canyon"),
    t("volcano", Nature, "a volcano"),
    t("palm tree", Nature, "a palm tree"),
    t("rose", Nature, "a rose"),
    t("lily", Nature, "a lily"),
    t("lavender", Nature, "lavender"),
    t("wildflowers", Nature, "wildflowers"),
    t("leaves", Nature, "leaves"),
    t("fire", Nature, "fire"),
    t("smoke", Nature, "smoke"),
    t("houseplant", Nature, "a houseplant"),
    t("moss", Nature, "moss"),
    // Moments and weather.
    o("night", Moment, "a photo taken at night."),
    o("sunset", Moment, "a photo of a sunset."),
    o("sunrise", Moment, "a photo of a sunrise at dawn."),
    o("rain", Moment, "a photo taken in the rain."),
    o("fog", Moment, "a foggy scene."),
    o("storm", Moment, "a stormy sky."),
    o("winter", Moment, "a winter scene."),
    o("christmas", Moment, "a christmas celebration."),
    o("wedding", Moment, "a wedding."),
    o("birthday", Moment, "a birthday party."),
    o("party", Moment, "people at a party."),
    o("concert", Moment, "a concert."),
    o("festival", Moment, "a festival."),
    o("sport", Moment, "people playing sport."),
    o("fireworks", Moment, "fireworks in the sky."),
    o("travel", Moment, "a travel photo."),
    o("hiking", Moment, "people hiking on a trail."),
    // Added 2026-11-13 with the tag head (Open Images classes).
    o("spring", Moment, "a photo taken in spring."),
    o("morning", Moment, "a photo taken in the morning."),
    o("sunlight", Moment, "a photo in bright sunlight."),
    o("vacation", Moment, "a vacation photo."),
    o("safari", Moment, "a safari photo."),
    o("camping", Moment, "a camping photo."),
    o("cooking", Moment, "a photo of someone cooking."),
    o("cycling", Moment, "a photo of people cycling."),
    o("skiing", Moment, "a photo of people skiing."),
    o("surfing", Moment, "a photo of people surfing."),
    o("football", Moment, "a photo of a football game."),
    o("martial arts", Moment, "a photo of martial arts."),
    o("horse riding", Moment, "a photo of horse riding."),
    o("motorsport", Moment, "a photo of a motor race."),
    o("performance", Moment, "a photo of a stage performance."),
    o("fishing", Moment, "a photo of people fishing."),
    // Framing and technique.
    o("close-up", Framing, "a close-up photo."),
    o("macro", Framing, "a macro photo of a tiny subject."),
    o("landscape", Framing, "a landscape photo."),
    o("cityscape", Framing, "a cityscape."),
    o("aerial view", Framing, "an aerial view from above."),
    o("panorama", Framing, "a panoramic photo."),
    o("silhouette", Framing, "a silhouette against the light."),
    o("reflection", Framing, "a reflection in the water."),
    o(
        "bokeh",
        Framing,
        "a photo with a blurry background and bokeh.",
    ),
    o(
        "long exposure",
        Framing,
        "a long exposure photo with light trails.",
    ),
    o("underwater", Framing, "an underwater photo."),
    o("symmetry", Framing, "a symmetrical composition."),
    o("minimalist", Framing, "a minimalist photo."),
    o("motion blur", Framing, "a photo with motion blur."),
    // Styles and media.
    o("photo", Style, "a real photograph."),
    o("black and white", Style, "a black and white photo."),
    o("sepia", Style, "a sepia toned photo."),
    o("vintage", Style, "a vintage photo."),
    o("film photo", Style, "a film photo with grain."),
    o("hdr", Style, "an hdr photo with oversaturated colors."),
    o("anime", Style, "an anime illustration."),
    o("manga", Style, "a black and white manga page."),
    o("cartoon", Style, "a cartoon."),
    o("comic", Style, "a comic book page."),
    o("illustration", Style, "a digital illustration."),
    o("digital art", Style, "digital art."),
    o("painting", Style, "an oil painting."),
    o("watercolor", Style, "a watercolor painting."),
    o("sketch", Style, "a pencil sketch."),
    o("pixel art", Style, "pixel art."),
    o("3d render", Style, "a 3d render."),
    o("concept art", Style, "concept art."),
    o("fantasy", Style, "a fantasy scene."),
    o("science fiction", Style, "a science fiction scene."),
    o("screenshot", Style, "a screenshot of a computer screen."),
    o("document", Style, "a scanned document with text."),
    o("meme", Style, "an internet meme with caption text."),
    o("logo", Style, "a logo."),
    o("chart", Style, "a chart or graph."),
    o("map", Style, "a map."),
    // Added 2026-11-13 with the tag head (Open Images classes).
    o("collage", Style, "a photo collage."),
    o("drawing", Style, "a drawing."),
];

/// SHA-256 of everything that shapes the embeddings: templates, names, groups and prompts, in
/// order. The embedded file carries the hash it was made from.
pub fn vocabulary_hash() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut put = |s: &str| {
        hasher.update((s.len() as u32).to_le_bytes());
        hasher.update(s.as_bytes());
    };
    put(THING_TEMPLATE);
    for word in VOCABULARY {
        put(word.name);
        put(&format!("{:?}", word.group));
        match word.prompts {
            Prompts::Thing(phrase) => put(phrase),
            Prompts::Own(sentence) => {
                put("own");
                put(sentence);
            }
        }
    }
    hasher.finalize().into()
}

/// The sentence embedded for `word`.
pub fn sentence(word: &Word) -> String {
    match word.prompts {
        Prompts::Thing(phrase) => THING_TEMPLATE.replace("{}", phrase),
        Prompts::Own(sentence) => sentence.to_string(),
    }
}

/// The word called `name` (exact, case-insensitive, surrounding spaces ignored).
pub fn find(name: &str) -> Option<usize> {
    let name = name.trim();
    VOCABULARY
        .iter()
        .position(|w| w.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_lower_case_and_plain() {
        let mut seen = HashSet::new();
        for word in VOCABULARY {
            assert!(seen.insert(word.name), "duplicate {}", word.name);
            assert_eq!(word.name, word.name.to_lowercase());
            assert_eq!(word.name, word.name.trim());
            assert!(!word.name.contains(',') && !word.name.contains('_'));
            assert!(!word.name.contains('\u{2014}') && !word.name.contains('\u{2013}')); // dash-ok
            assert!(sentence(word).ends_with('.'));
        }
        assert!(VOCABULARY.len() <= u16::MAX as usize);
    }

    #[test]
    fn templates_take_the_phrase() {
        let dog = &VOCABULARY[find("Dog ").unwrap()];
        assert_eq!(sentence(dog), "a photo of a dog.");
        assert_eq!(find("unknown word"), None);
    }
}
