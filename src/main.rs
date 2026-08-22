use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Write},
    net::SocketAddr,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use azalea::{
    JoinOpts,
    ecs::prelude::*,
    entity::{Dead, LocalEntity, Position, metadata::Player},
    pathfinder::{PathfinderOpts, goals::RadiusGoal},
    prelude::*,
    swarm::prelude::*,
};
use azalea_inventory::operations::ThrowClick;
use azalea_protocol::connect::Proxy;
use parking_lot::{Mutex, RwLock};
use rand::{Rng, seq::IndexedRandom};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    time::sleep,
};

const BANNER: &str = "Minecraft 26.2 MultiBot Pro v1.0";
const PROXY_SOURCE: &str = "https://api.proxyscrape.com/v2/?request=getproxies&protocol=socks5&timeout=10000&country=all&ssl=all&anonymity=all";
const DEFAULT_AUTH_PASSWORD: &str = "thematic";

#[derive(Clone, Debug)]
struct Config {
    server: String,
    bot_names: Vec<String>,
    stay_minutes: u64,
    join_gap: Duration,
    follow_radius: f64,
    follow_distance: f64,
    hit_distance: f64,
}

#[derive(Clone, Component)]
struct BotState {
    name: String,
    proxy: Option<Proxy>,
    stolen: bool,
    auth_deadline: Arc<Mutex<Option<Instant>>>,
    last_auth: Arc<Mutex<Option<(String, Instant)>>>,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            name: String::new(),
            proxy: None,
            stolen: false,
            auth_deadline: Arc::new(Mutex::new(None)),
            last_auth: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Clone, Resource)]
struct Controller {
    config: Arc<Config>,
    proxies: Arc<ProxyPool>,
    clients: Arc<RwLock<HashMap<String, Client>>>,
    queued_names: Arc<Mutex<VecDeque<(String, bool)>>>,
    known_names: Arc<Mutex<HashSet<String>>>,
    ai_enabled: Arc<AtomicBool>,
    hit_enabled: Arc<AtomicBool>,
    logs_enabled: Arc<AtomicBool>,
    auth_enabled: Arc<AtomicBool>,
    auth_password: Arc<RwLock<String>>,
    joining_enabled: Arc<AtomicBool>,
    infinite_spawn: Arc<AtomicBool>,
    maintain_target: Arc<AtomicUsize>,
    spam_generation: Arc<AtomicU64>,
    shutting_down: Arc<AtomicBool>,
    dead_bots: Arc<AtomicUsize>,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new(Config {
            server: "localhost".into(),
            bot_names: vec![],
            stay_minutes: 0,
            join_gap: Duration::from_millis(6500),
            follow_radius: 40.0,
            follow_distance: 2.0,
            hit_distance: 3.5,
        })
    }
}

impl Controller {
    fn new(config: Config) -> Self {
        let known = config
            .bot_names
            .iter()
            .map(|n| n.to_ascii_lowercase())
            .collect();
        Self {
            maintain_target: Arc::new(AtomicUsize::new(config.bot_names.len())),
            config: Arc::new(config),
            proxies: Arc::new(ProxyPool::default()),
            clients: Default::default(),
            queued_names: Default::default(),
            known_names: Arc::new(Mutex::new(known)),
            ai_enabled: Arc::new(AtomicBool::new(true)),
            hit_enabled: Arc::new(AtomicBool::new(true)),
            logs_enabled: Arc::new(AtomicBool::new(false)),
            auth_enabled: Arc::new(AtomicBool::new(true)),
            auth_password: Arc::new(RwLock::new(DEFAULT_AUTH_PASSWORD.into())),
            joining_enabled: Arc::new(AtomicBool::new(true)),
            infinite_spawn: Arc::new(AtomicBool::new(false)),
            spam_generation: Arc::new(AtomicU64::new(0)),
            shutting_down: Arc::new(AtomicBool::new(false)),
            dead_bots: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Default)]
struct ProxyPool {
    available: RwLock<Vec<Proxy>>,
    dead: Mutex<HashSet<SocketAddr>>,
    cursor: AtomicUsize,
}

impl ProxyPool {
    fn replace(&self, proxies: Vec<Proxy>) {
        *self.available.write() = proxies;
        self.cursor.store(0, Ordering::Relaxed);
    }

    fn next(&self) -> Option<Proxy> {
        let proxies = self.available.read();
        if proxies.is_empty() {
            return None;
        }
        for _ in 0..proxies.len() {
            let i = self.cursor.fetch_add(1, Ordering::Relaxed) % proxies.len();
            if !self.dead.lock().contains(&proxies[i].addr) {
                return Some(proxies[i].clone());
            }
        }
        None
    }

    fn mark_dead(&self, proxy: &Proxy) {
        self.dead.lock().insert(proxy.addr);
    }

    fn counts(&self) -> (usize, usize) {
        (self.available.read().len(), self.dead.lock().len())
    }
}

#[tokio::main]
async fn main() -> AppExit {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    println!("\n\x1b[36m=== {BANNER} ===\x1b[0m");
    println!("\x1b[35mCredits: Smile B | Native Minecraft Java 26.2\x1b[0m\n");

    let config = interactive_setup();
    let controller = Controller::new(config.clone());

    println!("\x1b[36mFetching public SOCKS5 proxies...\x1b[0m");
    match fetch_proxies().await {
        Ok(proxies) if !proxies.is_empty() => {
            println!("\x1b[32mLoaded {} SOCKS5 proxies.\x1b[0m", proxies.len());
            controller.proxies.replace(proxies);
        }
        Ok(_) | Err(_) => {
            println!(
                "\x1b[33mNo proxies loaded. Bots will use the direct connection until refresh succeeds.\x1b[0m"
            );
        }
    }

    let mut builder = SwarmBuilder::new()
        .set_handler(bot_handler)
        .set_swarm_handler(swarm_handler)
        .set_swarm_state(controller.clone())
        .join_delay(config.join_gap)
        .reconnect_after(None);

    for name in &config.bot_names {
        let proxy = controller.proxies.next();
        let state = BotState {
            name: name.clone(),
            proxy: proxy.clone(),
            stolen: false,
            ..Default::default()
        };
        let opts = join_opts(proxy);
        builder = builder.add_account_with_state_and_opts(Account::offline(name), state, opts);
    }

    builder.start(config.server.clone()).await
}

fn interactive_setup() -> Config {
    let server = ask_until("Server IP / hostname (include :port if needed): ", |v| {
        if v.trim().is_empty() {
            None
        } else {
            Some(v.trim().to_owned())
        }
    });

    let names_text = ask("Bot names separated by commas (blank = generated): ");
    let mut names: Vec<String> = names_text
        .split(',')
        .map(str::trim)
        .filter(|n| valid_name(n))
        .map(str::to_owned)
        .collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

    let count = ask_until("How many bots initially? (0 = infinite): ", |v| {
        v.trim().parse::<usize>().ok()
    });
    if names.is_empty() && count > 0 {
        names = (0..count).map(|_| generate_name()).collect();
    } else if count > names.len() {
        while names.len() < count {
            let name = generate_name();
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                names.push(name);
            }
        }
    } else if count > 0 {
        names.truncate(count);
    }

    let stay_minutes = ask_until("Minutes to stay (0 = until quit): ", |v| {
        v.trim().parse::<u64>().ok()
    });

    Config {
        server,
        bot_names: names,
        stay_minutes,
        join_gap: Duration::from_millis(6500),
        follow_radius: 40.0,
        follow_distance: 2.0,
        hit_distance: 3.5,
    }
}

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .expect("stdin unavailable");
    value.trim().to_owned()
}

fn ask_until<T>(prompt: &str, parse: impl Fn(&str) -> Option<T>) -> T {
    loop {
        let value = ask(prompt);
        if let Some(value) = parse(&value) {
            return value;
        }
        println!("\x1b[31mInvalid value. Try again.\x1b[0m");
    }
}

fn valid_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn generate_name() -> String {
    const WORDS: &[&str] = &[
        "Pixel", "Shadow", "Craft", "Steve", "Alex", "Boss", "Sniper", "Nova", "Ghost", "Block",
        "Ninja", "Gamer", "Creeper", "Turbo", "Dark", "Wolf",
    ];
    let mut rng = rand::rng();
    let word = WORDS.choose(&mut rng).copied().unwrap_or("Bot");
    match rng.random_range(0..4) {
        0 => format!("xX{word}Xx"),
        1 => format!("{word}_{}", rng.random_range(10..9999)),
        2 => format!("Itz{word}{}", rng.random_range(1..99)),
        _ => format!("{word}{}", rng.random_range(100..9999)),
    }
    .chars()
    .take(16)
    .collect()
}

async fn fetch_proxies() -> eyre::Result<Vec<Proxy>> {
    let body = reqwest::get(PROXY_SOURCE).await?.text().await?;
    let mut seen = HashSet::new();
    let proxies = body
        .lines()
        .filter_map(|line| SocketAddr::from_str(line.trim()).ok())
        .filter(|addr| seen.insert(*addr))
        .map(|addr| Proxy::new(addr, None))
        .collect();
    Ok(proxies)
}

fn join_opts(proxy: Option<Proxy>) -> JoinOpts {
    match proxy {
        Some(proxy) => JoinOpts::new().server_proxy(proxy),
        None => JoinOpts::new(),
    }
}

async fn bot_handler(bot: Client, event: Event, state: BotState) -> eyre::Result<()> {
    let controller = bot.resource::<Controller>();
    match event {
        Event::Login => {
            controller
                .clients
                .write()
                .insert(state.name.to_ascii_lowercase(), bot.clone());
            *state.auth_deadline.lock() = Some(Instant::now() + Duration::from_secs(30));
            println!(
                "\x1b[32m[{}] JOINED through {}\x1b[0m",
                state.name,
                proxy_label(&state.proxy)
            );
            if state.stolen {
                drop_inventory(&bot);
            }
        }
        Event::Chat(message) => {
            let text = message.message().to_string();
            if controller.logs_enabled.load(Ordering::Relaxed) {
                println!("\x1b[34m[CHAT -> {}]\x1b[0m {text}", state.name);
            }
            handle_auth(&bot, &state, &controller, &text);
        }
        Event::Tick => {
            ai_tick(&bot, &controller)?;
        }
        Event::Disconnect(reason) => {
            controller
                .clients
                .write()
                .remove(&state.name.to_ascii_lowercase());
            println!("\x1b[31m[{}] DISCONNECTED: {reason:?}\x1b[0m", state.name);
        }
        Event::ConnectionFailed(reason) => {
            controller
                .clients
                .write()
                .remove(&state.name.to_ascii_lowercase());
            println!(
                "\x1b[31m[{}] CONNECTION FAILED: {reason:?}\x1b[0m",
                state.name
            );
        }
        _ => {}
    }
    Ok(())
}

fn handle_auth(bot: &Client, state: &BotState, controller: &Controller, raw: &str) {
    if !controller.auth_enabled.load(Ordering::Relaxed)
        || state
            .auth_deadline
            .lock()
            .is_none_or(|end| Instant::now() > end)
    {
        return;
    }
    if raw.contains('<') && raw.contains('>') {
        return;
    }
    let text = raw.to_ascii_lowercase();
    let password = controller.auth_password.read().clone();
    let command =
        if text.contains("register") || text.contains("registration") || text.contains("/reg") {
            Some(format!("/register {password} {password}"))
        } else if text.contains("login") || text.contains("log in") || text.contains("/login") {
            Some(format!("/login {password}"))
        } else {
            None
        };
    if let Some(command) = command {
        let mut last = state.last_auth.lock();
        if last
            .as_ref()
            .is_some_and(|(old, when)| old == &command && when.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        bot.chat(&command);
        *last = Some((command, Instant::now()));
    }
}

fn ai_tick(bot: &Client, controller: &Controller) -> eyre::Result<()> {
    if !controller.ai_enabled.load(Ordering::Relaxed) || !bot.logged_in() {
        return Ok(());
    }
    let tick = bot.ticks_connected();
    if !tick.is_multiple_of(8) {
        return Ok(());
    }
    let eye = bot.eye_position()?;
    let target = bot
        .nearest_entity_by::<&Position, (With<Player>, Without<LocalEntity>, Without<Dead>)>(
            |position| eye.distance_to(**position) <= controller.config.follow_radius,
        )?;

    if let Some(target) = target {
        let distance = eye.distance_to(target.position()?);
        if controller.hit_enabled.load(Ordering::Relaxed)
            && distance <= controller.config.hit_distance
            && !bot.has_attack_cooldown()
        {
            target.look_at()?;
            target.attack();
        }
        if distance > controller.config.follow_distance + 0.75 && !bot.is_calculating_path() {
            bot.start_goto_with_opts(
                RadiusGoal::new(target.position()?, controller.config.follow_distance),
                PathfinderOpts::new()
                    .retry_on_no_path(false)
                    .max_timeout(Duration::from_secs(2)),
            );
        }
    } else if tick.is_multiple_of(100) && !bot.is_calculating_path() && !bot.is_executing_path() {
        let position = bot.position()?;
        let mut rng = rand::rng();
        let destination = position
            + azalea::Vec3::new(
                rng.random_range(-10.0..10.0),
                0.0,
                rng.random_range(-10.0..10.0),
            );
        bot.start_goto_with_opts(
            RadiusGoal::new(destination, 2.0),
            PathfinderOpts::new()
                .retry_on_no_path(false)
                .max_timeout(Duration::from_secs(3)),
        );
    }
    Ok(())
}

fn drop_inventory(bot: &Client) {
    if let Ok(Some(inventory)) = bot.open_inventory() {
        if let Some(menu) = inventory.menu().ok().flatten() {
            for slot in menu.player_slots_range() {
                if menu.slot(slot).is_some_and(|item| item.is_present()) {
                    inventory.click(ThrowClick::All { slot: slot as u16 });
                }
            }
        }
    }
}

async fn swarm_handler(
    swarm: Swarm,
    event: SwarmEvent,
    controller: Controller,
) -> eyre::Result<()> {
    match event {
        SwarmEvent::Init => {
            println!("\x1b[32mController ready. Type 'help' for commands.\x1b[0m");
            let command_swarm = swarm.clone();
            let command_controller = controller.clone();
            tokio::task::spawn_local(async move {
                command_loop(command_swarm, command_controller).await;
            });

            let refresh_controller = controller.clone();
            tokio::task::spawn_local(async move {
                proxy_refresh_loop(refresh_controller).await;
            });

            let queue_swarm = swarm.clone();
            let queue_controller = controller.clone();
            tokio::task::spawn_local(async move {
                queue_loop(queue_swarm, queue_controller).await;
            });

            if controller.config.stay_minutes > 0 {
                let exit_swarm = swarm.clone();
                let minutes = controller.config.stay_minutes;
                tokio::task::spawn_local(async move {
                    sleep(Duration::from_secs(minutes * 60)).await;
                    println!("Stay timer finished. Disconnecting all bots.");
                    exit_swarm.exit();
                });
            }
        }
        SwarmEvent::Disconnect(account, opts) => {
            if controller.shutting_down.load(Ordering::Relaxed) {
                return Ok(());
            }
            let name = account.username().to_owned();
            if let Some(proxy) = &opts.server_proxy {
                controller.proxies.mark_dead(proxy);
            }
            controller.dead_bots.fetch_add(1, Ordering::Relaxed);
            if controller.joining_enabled.load(Ordering::Relaxed) {
                controller.queued_names.lock().push_back((name, false));
            }
        }
        SwarmEvent::Chat(message) => {
            if controller.logs_enabled.load(Ordering::Relaxed) {
                println!("{}", message.message().to_ansi());
            }
        }
        _ => {}
    }
    Ok(())
}

async fn proxy_refresh_loop(controller: Controller) {
    loop {
        sleep(Duration::from_secs(60)).await;
        if controller.shutting_down.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(proxies) = fetch_proxies().await {
            if !proxies.is_empty() {
                let old = controller.proxies.available.read().len();
                controller.proxies.replace(proxies);
                let new = controller.proxies.available.read().len();
                if new != old {
                    println!("\x1b[36mProxy refresh: {new} available.\x1b[0m");
                }
            }
        }
    }
}

async fn queue_loop(swarm: Swarm, controller: Controller) {
    loop {
        sleep(Duration::from_millis(500)).await;
        if controller.shutting_down.load(Ordering::Relaxed) {
            return;
        }
        if controller.infinite_spawn.load(Ordering::Relaxed)
            && controller.joining_enabled.load(Ordering::Relaxed)
        {
            let name = unique_name(&controller);
            controller.queued_names.lock().push_back((name, false));
        }
        let next = if controller.joining_enabled.load(Ordering::Relaxed) {
            controller.queued_names.lock().pop_front()
        } else {
            None
        };
        let Some((name, stolen)) = next else { continue };
        let proxy = controller.proxies.next();
        let state = BotState {
            name: name.clone(),
            proxy: proxy.clone(),
            stolen,
            ..Default::default()
        };
        println!(
            "\x1b[36m[{name}] Connecting through {}...\x1b[0m",
            proxy_label(&proxy)
        );
        swarm
            .add_with_opts(&Account::offline(&name), state, &join_opts(proxy))
            .await;
        sleep(controller.config.join_gap).await;
    }
}

fn unique_name(controller: &Controller) -> String {
    loop {
        let name = generate_name();
        if controller
            .known_names
            .lock()
            .insert(name.to_ascii_lowercase())
        {
            return name;
        }
    }
}

fn proxy_label(proxy: &Option<Proxy>) -> String {
    proxy
        .as_ref()
        .map(|p| p.addr.to_string())
        .unwrap_or_else(|| "DIRECT".into())
}

async fn command_loop(swarm: Swarm, controller: Controller) {
    print_help();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if handle_command(&swarm, &controller, line).await {
            return;
        }
    }
}

async fn handle_command(swarm: &Swarm, c: &Controller, input: &str) -> bool {
    let (cmd, rest) = input.split_once(' ').unwrap_or((input, ""));
    match cmd.to_ascii_lowercase().as_str() {
        "all" => send_all(c, rest).await,
        "one" => {
            if let Some((name, msg)) = rest.split_once(' ') {
                if let Some(bot) = c.clients.read().get(&name.to_ascii_lowercase()).cloned() {
                    bot.chat(msg);
                } else {
                    println!("Bot '{name}' is not online.");
                }
            } else {
                println!("Use: one <name> <message>");
            }
        }
        "add" => match rest.parse::<usize>() {
            Ok(0) => {
                c.infinite_spawn.store(true, Ordering::Relaxed);
                c.joining_enabled.store(true, Ordering::Relaxed);
                println!("Infinite spawning enabled.");
            }
            Ok(count) => {
                c.maintain_target.fetch_add(count, Ordering::Relaxed);
                for _ in 0..count {
                    c.queued_names.lock().push_back((unique_name(c), false));
                }
                println!("Queued {count} bots.");
            }
            Err(_) => println!("Use: add <number> (0 = infinite)"),
        },
        "list" => {
            let (total, dead) = c.proxies.counts();
            println!(
                "Known: {} | Online: {} | Queued: {} | Proxies: {} | Dead proxies: {} | Target: {} | Infinite: {}",
                c.known_names.lock().len(),
                c.clients.read().len(),
                c.queued_names.lock().len(),
                total,
                dead,
                c.maintain_target.load(Ordering::Relaxed),
                c.infinite_spawn.load(Ordering::Relaxed)
            );
        }
        "spam" => {
            if let Some((ms, msg)) = rest.split_once(' ') {
                if let Ok(ms) = ms.parse::<u64>() {
                    if ms >= 1000 && !msg.is_empty() {
                        start_spam(c.clone(), ms, msg.to_owned());
                    } else {
                        println!("Interval must be >= 1000ms and message cannot be blank.");
                    }
                }
            } else {
                println!("Use: spam <interval_ms> <message>");
            }
        }
        "stopspam" => {
            c.spam_generation.fetch_add(1, Ordering::Relaxed);
            println!("Spam stopped.");
        }
        "stopspawn" => {
            c.infinite_spawn.store(false, Ordering::Relaxed);
            println!("Infinite spawning stopped. Existing queue remains.");
        }
        "stopjoin" => {
            c.infinite_spawn.store(false, Ordering::Relaxed);
            c.joining_enabled.store(false, Ordering::Relaxed);
            c.queued_names.lock().clear();
            println!("Joining stopped and queue cleared. Online bots remain.");
        }
        "startjoin" => {
            c.joining_enabled.store(true, Ordering::Relaxed);
            println!("Joining enabled.");
        }
        "rejoin" | "restart" => rejoin(c, rest),
        "steal" => steal_command(c, rest),
        "ai" => toggle(&c.ai_enabled, rest, "AI"),
        "hit" => toggle(&c.hit_enabled, rest, "Hitting"),
        "logs" => toggle(&c.logs_enabled, rest, "Logs"),
        "auth" => toggle(&c.auth_enabled, rest, "Auto-auth"),
        "authpass" => {
            if rest.is_empty() {
                println!("Use: authpass <password>");
            } else {
                *c.auth_password.write() = rest.to_owned();
                println!("Auth password updated for this run.");
            }
        }
        "resetban" | "resetall" => {
            c.proxies.dead.lock().clear();
            c.joining_enabled.store(true, Ordering::Relaxed);
            println!("Local stopped/dead-proxy state cleared. This does not bypass server bans.");
        }
        "resetplayer" => {
            if rest.is_empty() {
                println!("Use: resetplayer <name>");
            } else {
                c.known_names.lock().remove(&rest.to_ascii_lowercase());
                println!("Local state cleared for {rest}.");
            }
        }
        "version" => println!(
            "Native protocol: Minecraft Java 26.2 (Azalea 0.16). Runtime switching is not needed."
        ),
        "help" => print_help(),
        "quit" => {
            c.shutting_down.store(true, Ordering::Relaxed);
            c.spam_generation.fetch_add(1, Ordering::Relaxed);
            for bot in c.clients.read().values() {
                bot.disconnect();
            }
            swarm.exit();
            return true;
        }
        _ => println!("Unknown command. Type 'help'."),
    }
    false
}

async fn send_all(controller: &Controller, message: &str) {
    if message.is_empty() {
        println!("Use: all <message>");
        return;
    }
    let bots: Vec<Client> = controller.clients.read().values().cloned().collect();
    for bot in &bots {
        bot.chat(message);
        sleep(Duration::from_millis(1500)).await;
    }
    println!("Sent to {} online bots.", bots.len());
}

fn start_spam(controller: Controller, interval_ms: u64, message: String) {
    let generation = controller.spam_generation.fetch_add(1, Ordering::Relaxed) + 1;
    println!("Spam started every {interval_ms}ms.");
    tokio::task::spawn_local(async move {
        loop {
            if controller.spam_generation.load(Ordering::Relaxed) != generation {
                return;
            }
            send_all(&controller, &message).await;
            sleep(Duration::from_millis(interval_ms)).await;
        }
    });
}

fn rejoin(controller: &Controller, target: &str) {
    if target.eq_ignore_ascii_case("all") {
        let clients: Vec<(String, Client)> = controller
            .clients
            .read()
            .iter()
            .map(|(n, b)| (n.clone(), b.clone()))
            .collect();
        for (name, bot) in clients {
            bot.disconnect();
            controller.queued_names.lock().push_back((name, false));
        }
    } else if let Some(bot) = controller
        .clients
        .read()
        .get(&target.to_ascii_lowercase())
        .cloned()
    {
        bot.disconnect();
        controller
            .queued_names
            .lock()
            .push_back((target.to_owned(), false));
    } else {
        println!("Use: rejoin all OR rejoin <name>");
    }
}

fn steal_command(controller: &Controller, value: &str) {
    match value.to_ascii_lowercase().as_str() {
        "on" => {
            let mut added = 0;
            for bot in controller.clients.read().values() {
                if let Ok(tab) = bot.tab_list() {
                    for player in tab.values() {
                        let name = &player.profile.name;
                        if valid_name(name)
                            && controller
                                .known_names
                                .lock()
                                .insert(name.to_ascii_lowercase())
                        {
                            controller
                                .queued_names
                                .lock()
                                .push_back((name.clone(), true));
                            added += 1;
                        }
                    }
                }
            }
            println!("Steal scan queued {added} visible player names.");
        }
        "off" => println!("Steal scan is one-shot; no persistent mode is active."),
        "" => println!("Use: steal on/off OR steal <username>"),
        name if valid_name(name) => {
            if controller
                .known_names
                .lock()
                .insert(name.to_ascii_lowercase())
            {
                controller
                    .queued_names
                    .lock()
                    .push_back((value.to_owned(), true));
                println!("Queued stolen username: {value}");
            } else {
                println!("Name is already known/queued.");
            }
        }
        _ => println!("Invalid Minecraft username."),
    }
}

fn toggle(flag: &AtomicBool, value: &str, label: &str) {
    match value.to_ascii_lowercase().as_str() {
        "on" => {
            flag.store(true, Ordering::Relaxed);
            println!("{label} ON");
        }
        "off" => {
            flag.store(false, Ordering::Relaxed);
            println!("{label} OFF");
        }
        _ => println!("Use: {} on/off", label.to_ascii_lowercase()),
    }
}

fn print_help() {
    println!(
        r#"
=== BOT CONTROL ===
all <message>                 Send chat as every online bot
one <name> <message>          Send chat as one bot
list                          Show bot, queue and proxy counts
add <count>                   Add bots (0 = infinite generation)
stopspawn                     Stop infinite generation
stopjoin / startjoin          Pause or resume new connections
rejoin all | rejoin <name>    Reconnect with a new proxy
restart all | restart <name>  Alias of rejoin
spam <ms> <message>           Repeated chat, minimum 1000ms
stopspam                      Stop repeated chat
steal on                      Queue visible player usernames
steal <username>              Join as one specific username and drop inventory
ai on/off                     Toggle follow/wander AI
hit on/off                    Toggle nearby attacks
logs on/off                   Toggle server chat logs
auth on/off                   Toggle /register and /login detection
authpass <password>           Change runtime auth password
resetban / resetall           Clear local proxy stop state
resetplayer <name>            Clear local remembered-name state
version                       Show native protocol version
help                          Show this list
quit                          Disconnect and exit
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_valid() {
        for _ in 0..1000 {
            assert!(valid_name(&generate_name()));
        }
    }

    #[test]
    fn rejects_bad_names() {
        assert!(!valid_name("a"));
        assert!(!valid_name("has space"));
        assert!(!valid_name("way_too_long_username"));
    }
}
