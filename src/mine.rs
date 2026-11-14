use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use ureq;

// ==================== СТРУКТУРЫ ====================

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Statement {
    question: String,
    answer: String,
}

#[derive(Serialize, Deserialize, Default)]
struct KnowledgeBase {
    statements: Vec<Statement>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct WordEntry {
    meaning: String,
    pronoun: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Dictionary {
    words: HashMap<String, WordEntry>,
}

#[derive(Serialize, Deserialize, Default)]
struct Translations {
    pairs: HashMap<String, String>,
}

// ==================== БАЗА ЗНАНИЙ ====================

impl KnowledgeBase {
    fn load() -> Self {
        fs::read_to_string("stan_knowledge.json").ok().and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default()
    }
    fn save(&self) {
        fs::write("stan_knowledge.json", serde_json::to_string_pretty(self).unwrap()).ok();
    }
    fn learn(&mut self, q: &str, a: &str) {
        self.statements.push(Statement { question: q.trim().to_lowercase(), answer: a.trim().to_string() });
        self.save();
    }
    fn find_best(&self, input: &str) -> Option<&Statement> {
        let input_stems = tokenize_and_stem(input);
        if input_stems.is_empty() { return None; }
        let mut best: Option<(&Statement, f64)> = None;
        for st in &self.statements {
            let st_stems = tokenize_and_stem(&st.question);
            if st_stems.is_empty() { continue; }
            let mut common = 0.0;
            for in_word in &input_stems {
                for st_word in &st_stems {
                    if levenshtein(in_word, st_word) <= 2 { common += 1.0; break; }
                }
            }
            if common == 0.0 { continue; }
            let score = (common / (input_stems.len() as f64)) * (common / (st_stems.len() as f64));
            if score > 0.4 {
                match best { Some((_, s)) if s >= score => {}, _ => best = Some((st, score)) }
            }
        }
        best.map(|(st, _)| st)
    }
    fn edit_statement(&mut self, index: usize, new_q: &str, new_a: &str) -> bool {
        if index < self.statements.len() {
            if !new_q.trim().is_empty() { self.statements[index].question = new_q.trim().to_lowercase(); }
            if !new_a.trim().is_empty() { self.statements[index].answer = new_a.trim().to_string(); }
            self.save(); true
        } else { false }
    }
}

// ==================== БИБЛИОТЕКА СЛОВ ====================

impl Dictionary {
    fn load() -> Self {
        fs::read_to_string("stan_dictionary.json").ok().and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default()
    }
    fn save(&self) {
        fs::write("stan_dictionary.json", serde_json::to_string_pretty(self).unwrap()).ok();
    }
    fn add_word(&mut self, w: &str, m: &str, p: &str) {
        self.words.insert(w.trim().to_lowercase(), WordEntry { meaning: m.trim().to_string(), pronoun: normalize_pronoun(p) });
        self.save();
    }
    fn edit_word(&mut self, old: &str, new_w: &str, new_m: &str, new_p: &str) -> bool {
        let old_lower = old.trim().to_lowercase();
        if self.words.contains_key(&old_lower) {
            self.words.remove(&old_lower);
            self.words.insert(new_w.trim().to_lowercase(), WordEntry { meaning: new_m.trim().to_string(), pronoun: normalize_pronoun(new_p) });
            self.save(); true
        } else { false }
    }
    fn generate_text(&self, topic: &str) -> String {
        let topic_stem = stem_word(&topic.to_lowercase());
        let mut matched = Vec::new();
        for (word, entry) in &self.words {
            if stem_word(word).contains(&topic_stem) || entry.meaning.to_lowercase().contains(&topic.to_lowercase()) {
                matched.push((word.clone(), entry.clone()));
            }
        }
        if matched.is_empty() { return format!("Я не нашел в своей библиотеке слов ничего про «{}».", topic); }
        let mut text = format!("Вот что я знаю про «{}»:\n\n", topic);
        for (i, (word, entry)) in matched.iter().enumerate() {
            text.push_str(&format!("{}. {} ({}) — это {}\n", i + 1, capitalize(word), entry.pronoun, entry.meaning));
        }
        text
    }
}

// ==================== ПЕРЕВОДЧИК ====================

impl Translations {
    fn load() -> Self {
        fs::read_to_string("stan_translations.json").ok().and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default()
    }
    fn save(&self) {
        fs::write("stan_translations.json", serde_json::to_string_pretty(self).unwrap()).ok();
    }
    fn add_pair(&mut self, w1: &str, w2: &str) {
        let (a, b) = (w1.trim().to_lowercase(), w2.trim().to_lowercase());
        if !a.is_empty() && !b.is_empty() { self.pairs.insert(a.clone(), b.clone()); self.pairs.insert(b, a); self.save(); }
    }
    fn translate(&self, word: &str) -> Option<&String> {
        let w = word.trim().to_lowercase();
        let stem = stem_word(&w);
        if let Some(tr) = self.pairs.get(&w) { return Some(tr); }
        for (key, val) in &self.pairs { if stem_word(key) == stem { return Some(val); } }
        None
    }
}

// ==================== ПОИСК В ИНТЕРНЕТЕ ====================

#[derive(Deserialize)]
struct WikiResponse { query: Option<WikiQuery> }
#[derive(Deserialize)]
struct WikiQuery { pages: HashMap<String, WikiPage> }
#[derive(Deserialize)]
struct WikiPage { extract: Option<String> }

fn search_web(query: &str) -> Option<String> {
    let clean_query = query.to_lowercase().replace("что такое", "").replace("кто такой", "").replace("кто такая", "").replace("расскажи про", "").replace("расскажи о", "").trim().to_string();
    if clean_query.is_empty() { return None; }

    let url = format!("https://ru.wikipedia.org/w/api.php?action=query&format=json&prop=extracts&exintro=1&explaintext=1&titles={}&utf8=1", url_encode(&clean_query));

    match ureq::get(&url).call() {
        Ok(response) => {
            if let Ok(wiki_resp) = response.into_json::<WikiResponse>() {
                if let Some(query_data) = wiki_resp.query {
                    for (_, page) in query_data.pages {
                        if let Some(extract) = page.extract {
                            if !extract.is_empty() {
                                let first_sentence = extract.split(". ").next().unwrap_or(&extract);
                                return Some(format!("Согласно Википедии, {}", first_sentence.trim()));
                            }
                        }
                    }
                }
            }
            None
        }
        Err(_) => None,
    }
}

fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for b in s.as_bytes() {
        match *b {
            b' ' => result.push_str("%20"),
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => result.push(*b as char),
            _ => result.push_str(&format!("%{:02X}", b)),
        }
    }
    result
}

// ==================== МАТЕМАТИКА ====================

fn try_solve_math(input: &str) -> Option<String> {
    let cleaned: String = input.to_lowercase().chars().filter(|c| c.is_numeric() || "+-*/()., ".contains(*c)).collect();
    let expr = cleaned.trim();
    if expr.is_empty() || !expr.chars().any(|c| c.is_numeric()) || (!expr.contains('+') && !expr.contains('-') && !expr.contains('*') && !expr.contains('/')) { return None; }

    struct Parser<'a> { chars: Vec<char>, pos: usize, _expr: &'a str }
    impl<'a> Parser<'a> {
        fn new(s: &'a str) -> Self { Self { chars: s.chars().collect(), pos: 0, _expr: s } }
        fn parse_expression(&mut self) -> Result<f64, String> {
            let mut result = self.parse_term()?;
            while let Some(c) = self.peek() {
                if c == '+' || c == '-' {
                    self.pos += 1; let rhs = self.parse_term()?;
                    if c == '+' { result += rhs; } else { result -= rhs; }
                } else { break; }
            }
            Ok(result)
        }
        fn parse_term(&mut self) -> Result<f64, String> {
            let mut result = self.parse_factor()?;
            while let Some(c) = self.peek() {
                if c == '*' || c == '/' {
                    self.pos += 1; let rhs = self.parse_factor()?;
                    if c == '*' { result *= rhs; } else {
                        if rhs == 0.0 { return Err("деление на ноль".to_string()); }
                        result /= rhs;
                    }
                } else { break; }
            }
            Ok(result)
        }
        fn parse_factor(&mut self) -> Result<f64, String> {
            self.skip_spaces();
            if let Some(c) = self.peek() {
                if c == '(' { self.pos += 1; let res = self.parse_expression()?; self.skip_spaces(); if self.peek() == Some(')') { self.pos += 1; } return Ok(res); }
                if c == '-' { self.pos += 1; return Ok(-self.parse_factor()?); }
            }
            self.parse_number()
        }
        fn parse_number(&mut self) -> Result<f64, String> {
            self.skip_spaces(); let start = self.pos;
            while let Some(c) = self.peek() { if c.is_numeric() || c == '.' { self.pos += 1; } else { break; } }
            if start == self.pos { return Err("ожидалось число".to_string()); }
            self.chars[start..self.pos].iter().collect::<String>().parse::<f64>().map_err(|_| "ошибка".to_string())
        }
        fn skip_spaces(&mut self) { while let Some(c) = self.peek() { if c.is_whitespace() { self.pos += 1; } else { break; } } }
        fn peek(&self) -> Option<char> { self.chars.get(self.pos).copied() }
    }

    let mut parser = Parser::new(expr);
    match parser.parse_expression() {
        Ok(r) => Some(format!("{} = {}", expr, if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{}", r) })),
        Err(_) => None,
    }
}

// ==================== ГЕНЕРАТОР АРТА ====================

fn generate_pixel_art(prompt: &str) -> String {
    let prompt_lower = prompt.to_lowercase();
    let width = 40; let height = 20;
    let mut canvas = vec![vec![' '; width]; height];

    if prompt_lower.contains("ночь") || prompt_lower.contains("космос") || prompt_lower.contains("звезд") {
        for y in 0..height { for x in 0..width { if (x * y + x + y) % 7 == 0 { canvas[y][x] = '·'; } if (x * 3 + y * 5) % 13 == 0 { canvas[y][x] = '✦'; } } }
    } else if prompt_lower.contains("вода") || prompt_lower.contains("море") {
        for y in 0..height { for x in 0..width { if ((x as f32 * 0.2).sin() + (y as f32 * 0.5).cos()).abs() > 0.5 { canvas[y][x] = '~'; } } }
    } else if prompt_lower.contains("трава") || prompt_lower.contains("лес") {
        for y in 15..height { for x in 0..width { if (x + y) % 2 == 0 { canvas[y][x] = '"'; } } }
    }

    let cx = width / 2; let cy = height / 2;

    if prompt_lower.contains("кот") || prompt_lower.contains("кошка") {
        canvas[cy - 5][cx - 4] = '^'; canvas[cy - 6][cx - 3] = '^'; canvas[cy - 5][cx + 4] = '^'; canvas[cy - 6][cx + 3] = '^';
        canvas[cy - 2][cx - 4] = 'O'; canvas[cy - 2][cx + 3] = 'O'; canvas[cy + 1][cx] = 'v';
    } else if prompt_lower.contains("дом") || prompt_lower.contains("здание") {
        for i in 0..5 { for j in 0..(i * 2 + 1) { if cx >= i { canvas[cy - 5 + i][cx - i + j] = '^'; } } }
        for i in 0..6 { for j in 0..7 { canvas[cy - 1 + i][cx - 3 + j] = '|'; } }
        canvas[cy + 1][cx - 1] = 'O'; canvas[cy + 2][cx + 1] = 'O';
    } else if prompt_lower.contains("сердце") || prompt_lower.contains("любовь") {
        let pattern = ["  ***   ***  ", " ***** ***** ", "*************", " *********** ", "  *********  ", "   *******   ", "    *****    ", "     ***     ", "      *      "];
        for (i, line) in pattern.iter().enumerate() { for (j, ch) in line.chars().enumerate() { if ch == '*' && cx >= 6 { canvas[cy - 4 + i][cx - 6 + j] = '♥'; } } }
    } else {
        for y in 0..height { for x in 0..width { let dx = x as f32 - cx as f32; let dy = (y as f32 - cy as f32) * 2.0; let dist = (dx * dx + dy * dy).sqrt(); if dist < 6.0 { canvas[y][x] = '@'; } else if dist < 8.0 { canvas[y][x] = '.'; } } }
    }

    let mut result = format!("\x1b[1m🎨 Арт: «{}»\x1b[0m\n\n", prompt);
    for y in 0..height {
        result.push_str("  ");
        for x in 0..width {
            let ch = canvas[y][x];
            let color = match ch {
                'O' => "\x1b[91m", '♥' => "\x1b[95m", '@' => "\x1b[94m", '^' => "\x1b[92m", '"' => "\x1b[32m", '~' => "\x1b[96m", '✦' => "\x1b[93m", '·' => "\x1b[90m", 'v' => "\x1b[35m", '|' => "\x1b[33m", _ => "\x1b[0m",
            };
            result.push_str(&format!("{}{}\x1b[0m", color, ch));
        }
        result.push('\n');
    }
    result
}

// ==================== МОРФОЛОГИЯ И УТИЛИТЫ ====================

fn stem_word(word: &str) -> String {
    let mut w = word.to_string();
    if w.len() < 4 { return w; }
    let endings = ["ами", "ями", "ого", "его", "ому", "ему", "ыми", "ими", "ая", "яя", "ое", "ее", "ые", "ие", "ой", "ей", "ую", "юю", "ах", "ях", "ов", "ев", "ам", "ям", "ом", "ем", "ий", "ый", "а", "я", "о", "е", "ы", "и", "у", "ю", "ь"];
    for ending in endings { if w.ends_with(ending) && w.len() - ending.len() >= 3 { w.truncate(w.len() - ending.len()); break; } }
    w
}

fn tokenize_and_stem(text: &str) -> HashSet<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2).map(|w| stem_word(w)).collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() { Some(f) => f.to_uppercase().collect::<String>() + c.as_str(), None => String::new() }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a_chars, b_chars) = (a.chars().collect::<Vec<_>>(), b.chars().collect::<Vec<_>>());
    let (a_len, b_len) = (a_chars.len(), b_chars.len());
    if a_len == 0 { return b_len; } if b_len == 0 { return a_len; }
    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr: Vec<usize> = vec![0; b_len + 1];
    for i in 1..=a_len {
        curr[0] = i;
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_len]
}

fn normalize_pronoun(p: &str) -> String {
    let p = p.trim().to_lowercase();
    match p.as_str() {
        "я" | "меня" | "мне" | "мной" | "мною" => "я".to_string(), "ты" | "тебя" | "тебе" | "тобой" | "тобою" => "ты".to_string(),
        "он" | "его" | "него" | "ему" | "нему" | "им" | "ним" => "он".to_string(), "она" | "её" | "ее" | "неё" | "нее" | "ей" | "ней" | "ею" | "нею" => "она".to_string(),
        "оно" => "оно".to_string(), "мы" | "нас" | "нам" | "нами" => "мы".to_string(),
        "вы" | "вас" | "вам" | "вами" => "вы".to_string(), "они" | "их" | "них" | "ими" | "ними" => "они".to_string(),
        _ => p,
    }
}

fn is_pronoun(word: &str) -> bool {
    let w = word.trim().to_lowercase();
    let pronouns = ["я", "меня", "мне", "мной", "мною", "ты", "тебя", "тебе", "тобой", "тобою", "он", "его", "него", "ему", "нему", "им", "ним", "она", "её", "ее", "неё", "нее", "ей", "ней", "ею", "нею", "оно", "мы", "нас", "нам", "нами", "вы", "вас", "вам", "вами", "они", "их", "них", "им", "ним", "ими", "ними"];
    pronouns.contains(&w.as_str())
}

fn normalize_input_with_pronouns(input: &str) -> String {
    let mut result = String::new();
    for word in input.split_whitespace() {
        let lower = word.to_lowercase();
        if is_pronoun(&lower) { result.push_str(&normalize_pronoun(&lower)); } else { result.push_str(word); }
        result.push(' ');
    }
    result.trim().to_string()
}

// ==================== ДИЗАЙН ====================

fn print_bot_message(text: &str) {
    println!("\n\x1b[36m╭─ STAN GPT ─────────────────────────────╮\x1b[0m");
    for line in text.lines() { println!("\x1b[36m│\x1b[0m {}", line); }
    println!("\x1b[36m╰────────────────────────────────────────╯\x1b[0m\n");
}

fn print_user_prompt() { print!("\x1b[32mВы ❯ \x1b[0m"); io::stdout().flush().ok(); }

fn print_menu() {
    println!("\x1b[90m┌─────────────────────────────── КОМАНДЫ ───────────────────────────────┐");
    println!("│ \x1b[36m/1\x1b[90m — Обучить (вопрос/ответ)     │ \x1b[36m/2\x1b[90m — Добавить слово        │");
    println!("│ \x1b[36m/3\x1b[90m — Исправить слово            │ \x1b[36m/4\x1b[90m — Добавить перевод      │");
    println!("│ \x1b[36m/5\x1b[90m — Перевести слово            │ \x1b[36m/6\x1b[90m — Составить текст       │");
    println!("│ \x1b[36m/7\x1b[90m — Показать знания            │ \x1b[36m/8\x1b[90m — Показать слова        │");
    println!("│ \x1b[36m/9\x1b[90m — Помощь                     │ \x1b[36m/0\x1b[90m — Выход                 │");
    println!("│ \x1b[36m/10\x1b[90m — 🎨 Арт                     │ \x1b[36m/11\x1b[90m — ✏️ Исправить вопрос/ответ│");
    println!("└────────────────────────────────────────────────────────────────────────┘\x1b[0m\n");
}

// ==================== MAIN ====================

fn main() {
    print!("\x1b[2J\x1b[1;1H");
    println!("\x1b[1;36m");
    println!("  ███████╗████████╗ █████╗ ███╗   ██╗     ██████╗ ██████╗ ████████╗");
    println!("  ██╔════╝╚══██╔══╝██╔══██╗████╗  ██║    ██╔════╝ ██╔══██╗╚══██╔══╝");
    println!("  ███████╗   ██║   ███████║██╔██╗ ██║    ██║  ███╗██████╔╝   ██║   ");
    println!("  ╚════██║   ██║   ██╔══██║██║╚██╗██║    ██║   ██║██╔═══╝    ██║   ");
    println!("  ███████║   ██║   ██║  ██║██║ ╚████║    ╚██████╔╝██║        ██║   ");
    println!("  ╚══════╝   ╚═╝   ╚═╝  ╚═╝╚═╝  ╚═══╝     ╚═════╝ ╚═╝        ╚═╝   ");
    println!("\x1b[0m");
    println!("\x1b[90m  Твой локальный ИИ на Rust. Версия 2.0 (с интернетом)\x1b[0m\n");

    let mut kb = KnowledgeBase::load();
    let mut dict = Dictionary::load();
    let mut trans = Translations::load();

    print_bot_message(&format!(
        "Привет! Я STAN GPT 🤖\n\nВ памяти: {} утверждений, {} слов, {} переводов.\nТеперь я умею: искать в Википедии, если не знаю ответа.\nСправка: /9",
        kb.statements.len(), dict.words.len(), trans.pairs.len() / 2
    ));
    print_menu();

    loop {
        print_user_prompt();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() { break; }
        let input = input.trim();
        if input.is_empty() { continue; }

        let command = match input {
            "/0" | "/exit" => "/0", "/1" | "/learn" => "/1", "/2" | "/word" => "/2", "/3" | "/edit" => "/3",
            "/4" | "/translate_add" => "/4", "/5" | "/translate" => "/5", "/6" | "/generate" => "/6",
            "/7" | "/list" => "/7", "/8" | "/dictionary" => "/8", "/9" | "/help" => "/9", "/10" | "/img" => "/10",
            "/11" | "/edit_qa" => "/11",
            _ => "",
        };

        if !command.is_empty() {
            match command {
                "/0" => { print_bot_message("До встречи! 👋"); break; }
                "/9" => { print_menu(); continue; }
                "/7" => {
                    if kb.statements.is_empty() { print_bot_message("База знаний пуста."); }
                    else {
                        let mut text = String::from("Мои знания:\n");
                        for (i, s) in kb.statements.iter().enumerate() { text.push_str(&format!("  {}. «{}» → {}\n", i + 1, s.question, s.answer)); }
                        print_bot_message(&text);
                    }
                    continue;
                }
                "/8" => {
                    if dict.words.is_empty() { print_bot_message("Библиотека слов пуста."); }
                    else {
                        let mut text = format!("Моя библиотека ({} шт.):\n", dict.words.len());
                        for (word, entry) in &dict.words {
