//! # Satranç — insana karşı bir arama motoru
//!
//! Tam kurallarıyla satranç: rok, geçerken alma, terfi, şah, mat, pat; elli hamle kuralı, üç kez
//! tekrar ve yetersiz malzemeyle beraberlik. Rakip bu dosyanın kendi küçük motoru: alfa-beta
//! negamax, alışlarda sakinleşme araması (quiescence), MVV-LVA sıralaması, malzeme ve kare
//! tabloları. Motorun otuz yedinci oyunu.
//!
//! | girdi | iş |
//! |---|---|
//! | **tıkla** kendi taşına, sonra hedef kareye | hamle (terfi vezire; terfiden önce **R**/**B**/**N** basılıysa kale, fil, at) |
//! | **U** | son hamle çiftini geri al |
//! | **M** | motor iki taraf için de oynasın / bıraksın (WASD kameranın) |
//! | **Y** | yeniden başla |
//!
//! ## Kurallar doğru mu: perft
//!
//! `satranc --test` önce kural üretecini ölçüyor: bilinen konumlardan belli derinliğe kadar
//! oynanabilecek bütün hamle dizilerini sayıyor ve satranç programlamasının herkesçe bilinen
//! sayılarıyla karşılaştırıyor — başlangıç konumu 4 derinlikte 197 281; rok, geçerken alma ve
//! terfinin hepsini barındıran "Kiwipete" 3 derinlikte 97 862; açmaz ve geçerken almanın tuzağı
//! olan üçüncü konum 4 derinlikte 43 238. Tek bir kural hatası bu sayıları bozar. Sonra motorun
//! bir hamlede matı bulduğunu, ve son olarak iki tarafı da motorun oynadığı bir maçı pencere
//! açmadan baştan sona oynatıyor: her taş kendi karesinde durmalı, tahtadaki taş sayısı sahnedeki
//! taş sayısı olmalı.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! Hiçbiri. Taşlar primitiflerden kurulu (`MeshSource` + hiyerarşi: bir ebeveyn, onu izleyen
//! parçalar), hamleler `Tween`, kare seçimi `gizmo::pointer::cursor_on_plane`, HUD `Text`.
//!
//! ## Ortam değişkenleri
//!   * `SATRANC_OTO=1` — motor iki taraf için oynar
//!   * `SATRANC_DERINLIK=<n>` — arama derinliği (varsayılan 3)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::renderer::tween::{Ease, Tween};
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};

// =============================================================================================
// Kurallar
// =============================================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Side {
    White,
    Black,
}

impl Side {
    fn other(self) -> Side {
        match self {
            Side::White => Side::Black,
            Side::Black => Side::White,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Kind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl Kind {
    fn value(self) -> i32 {
        match self {
            Kind::Pawn => 100,
            Kind::Knight => 320,
            Kind::Bishop => 330,
            Kind::Rook => 500,
            Kind::Queen => 900,
            Kind::King => 20_000,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
struct Piece {
    side: Side,
    kind: Kind,
}

/// Bir kare: `rank * 8 + file`, `0` = a1, `63` = h8.
type Sq = u8;

fn sq(file: i32, rank: i32) -> Option<Sq> {
    ((0..8).contains(&file) && (0..8).contains(&rank)).then(|| (rank * 8 + file) as Sq)
}
fn file_of(s: Sq) -> i32 {
    i32::from(s % 8)
}
fn rank_of(s: Sq) -> i32 {
    i32::from(s / 8)
}
fn name(s: Sq) -> String {
    format!("{}{}", (b'a' + s % 8) as char, s / 8 + 1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
struct Move {
    from: Sq,
    to: Sq,
    promo: Option<Kind>,
}

const WK: u8 = 1;
const WQ: u8 = 2;
const BK: u8 = 4;
const BQ: u8 = 8;

/// Bir konum: kim oynuyor, rok hakları, geçerken alınabilecek kare, elli hamle sayacı.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
struct Pos {
    board: [Option<Piece>; 64],
    turn: Side,
    castle: u8,
    ep: Option<Sq>,
    halfmove: u16,
}

const KNIGHT: [(i32, i32); 8] = [(1, 2), (2, 1), (2, -1), (1, -2), (-1, -2), (-2, -1), (-2, 1), (-1, 2)];
const KING: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
const ROOK_DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const BISHOP_DIRS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

impl Pos {
    fn start() -> Pos {
        Pos::fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").expect("başlangıç konumu")
    }

    /// Forsyth–Edwards gösterimi: testlerin bilinen konumları bununla kuruluyor.
    fn fen(text: &str) -> Option<Pos> {
        let mut parts = text.split_whitespace();
        let mut board = [None; 64];
        for (row, line) in parts.next()?.split('/').enumerate() {
            let rank = 7 - row as i32;
            let mut file = 0;
            for c in line.chars() {
                if let Some(d) = c.to_digit(10) {
                    file += d as i32;
                    continue;
                }
                let side = if c.is_ascii_uppercase() { Side::White } else { Side::Black };
                let kind = match c.to_ascii_lowercase() {
                    'p' => Kind::Pawn,
                    'n' => Kind::Knight,
                    'b' => Kind::Bishop,
                    'r' => Kind::Rook,
                    'q' => Kind::Queen,
                    'k' => Kind::King,
                    _ => return None,
                };
                board[sq(file, rank)? as usize] = Some(Piece { side, kind });
                file += 1;
            }
        }
        let turn = if parts.next()? == "w" { Side::White } else { Side::Black };
        let castle = parts.next()?.chars().fold(0, |c, ch| {
            c | match ch {
                'K' => WK,
                'Q' => WQ,
                'k' => BK,
                'q' => BQ,
                _ => 0,
            }
        });
        let ep = match parts.next()? {
            "-" => None,
            s => {
                let b = s.as_bytes();
                sq(i32::from(b[0]) - i32::from(b'a'), i32::from(b[1]) - i32::from(b'1'))
            }
        };
        let halfmove = parts.next().and_then(|h| h.parse().ok()).unwrap_or(0);
        Some(Pos { board, turn, castle, ep, halfmove })
    }

    fn king(&self, side: Side) -> Option<Sq> {
        (0..64).find(|&s| self.board[s as usize] == Some(Piece { side, kind: Kind::King }))
    }

    /// `s` karesine `by` tarafından saldırılıyor mu.
    fn attacked(&self, s: Sq, by: Side) -> bool {
        let (f, r) = (file_of(s), rank_of(s));
        let at = |f: i32, r: i32| sq(f, r).and_then(|t| self.board[t as usize]);
        let is = |p: Option<Piece>, kinds: &[Kind]| p.is_some_and(|p| p.side == by && kinds.contains(&p.kind));
        // Piyonlar: saldıran piyon, bu karenin saldıranın tarafından bakınca bir geri çaprazında.
        let back = if by == Side::White { -1 } else { 1 };
        if is(at(f - 1, r + back), &[Kind::Pawn]) || is(at(f + 1, r + back), &[Kind::Pawn]) {
            return true;
        }
        if KNIGHT.iter().any(|(df, dr)| is(at(f + df, r + dr), &[Kind::Knight])) {
            return true;
        }
        if KING.iter().any(|(df, dr)| is(at(f + df, r + dr), &[Kind::King])) {
            return true;
        }
        for (dirs, kinds) in [(ROOK_DIRS, [Kind::Rook, Kind::Queen]), (BISHOP_DIRS, [Kind::Bishop, Kind::Queen])] {
            for (df, dr) in dirs {
                let (mut x, mut y) = (f + df, r + dr);
                while let Some(t) = sq(x, y) {
                    if let Some(p) = self.board[t as usize] {
                        if p.side == by && kinds.contains(&p.kind) {
                            return true;
                        }
                        break;
                    }
                    x += df;
                    y += dr;
                }
            }
        }
        false
    }

    fn in_check(&self, side: Side) -> bool {
        self.king(side).is_some_and(|k| self.attacked(k, side.other()))
    }

    /// Kurallara uyan, ama şahı açıkta bırakıp bırakmadığına bakılmamış hamleler.
    fn pseudo_moves(&self, out: &mut Vec<Move>) {
        let me = self.turn;
        for from in 0..64u8 {
            let Some(p) = self.board[from as usize] else { continue };
            if p.side != me {
                continue;
            }
            let (f, r) = (file_of(from), rank_of(from));
            let push = |to: Sq, out: &mut Vec<Move>| out.push(Move { from, to, promo: None });
            match p.kind {
                Kind::Pawn => {
                    let (dir, start, last) = if me == Side::White { (1, 1, 7) } else { (-1, 6, 0) };
                    let promote = |to: Sq, out: &mut Vec<Move>| {
                        if rank_of(to) == last {
                            for k in [Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight] {
                                out.push(Move { from, to, promo: Some(k) });
                            }
                        } else {
                            out.push(Move { from, to, promo: None });
                        }
                    };
                    if let Some(one) = sq(f, r + dir).filter(|t| self.board[*t as usize].is_none()) {
                        promote(one, out);
                        if r == start {
                            if let Some(two) = sq(f, r + 2 * dir).filter(|t| self.board[*t as usize].is_none()) {
                                out.push(Move { from, to: two, promo: None });
                            }
                        }
                    }
                    for df in [-1, 1] {
                        if let Some(t) = sq(f + df, r + dir) {
                            let enemy = self.board[t as usize].is_some_and(|q| q.side != me);
                            if enemy || self.ep == Some(t) {
                                promote(t, out);
                            }
                        }
                    }
                }
                Kind::Knight | Kind::King => {
                    let steps = if p.kind == Kind::Knight { KNIGHT } else { KING };
                    for (df, dr) in steps {
                        if let Some(t) = sq(f + df, r + dr) {
                            if self.board[t as usize].is_none_or(|q| q.side != me) {
                                push(t, out);
                            }
                        }
                    }
                }
                Kind::Bishop | Kind::Rook | Kind::Queen => {
                    let dirs: &[(i32, i32)] = match p.kind {
                        Kind::Bishop => &BISHOP_DIRS,
                        Kind::Rook => &ROOK_DIRS,
                        _ => &[(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)],
                    };
                    for (df, dr) in dirs {
                        let (mut x, mut y) = (f + df, r + dr);
                        while let Some(t) = sq(x, y) {
                            match self.board[t as usize] {
                                None => push(t, out),
                                Some(q) => {
                                    if q.side != me {
                                        push(t, out);
                                    }
                                    break;
                                }
                            }
                            x += df;
                            y += dr;
                        }
                    }
                }
            }
        }
        // Rok: hakkı var, aradaki kareler boş, şah çekilmemiş, geçtiği kareler saldırı altında değil.
        let (rank, kside, qside) = if me == Side::White { (0, WK, WQ) } else { (7, BK, BQ) };
        let king_at = sq(4, rank).expect("e sütunu");
        if self.board[king_at as usize] == Some(Piece { side: me, kind: Kind::King }) && !self.attacked(king_at, me.other()) {
            let empty = |files: &[i32]| files.iter().all(|&f| self.board[sq(f, rank).expect("kare") as usize].is_none());
            let safe = |files: &[i32]| files.iter().all(|&f| !self.attacked(sq(f, rank).expect("kare"), me.other()));
            let rook = |f: i32| self.board[sq(f, rank).expect("kare") as usize] == Some(Piece { side: me, kind: Kind::Rook });
            if self.castle & kside != 0 && rook(7) && empty(&[5, 6]) && safe(&[5, 6]) {
                out.push(Move { from: king_at, to: sq(6, rank).expect("g"), promo: None });
            }
            if self.castle & qside != 0 && rook(0) && empty(&[1, 2, 3]) && safe(&[2, 3]) {
                out.push(Move { from: king_at, to: sq(2, rank).expect("c"), promo: None });
            }
        }
    }

    /// Hamlenin ardından gelen konum. Hamlenin kurala uygun olduğunu varsayar.
    fn play(&self, m: Move) -> Pos {
        let mut n = *self;
        let piece = n.board[m.from as usize].take().expect("kaynak karede bir taş");
        let captured = n.board[m.to as usize];
        let mut capture = captured.is_some();
        if piece.kind == Kind::Pawn && Some(m.to) == self.ep {
            // Geçerken alma: alınan piyon hedefin bir gerisinde.
            let behind = sq(file_of(m.to), rank_of(m.from)).expect("piyonun karesi");
            n.board[behind as usize] = None;
            capture = true;
        }
        n.board[m.to as usize] = Some(Piece { side: piece.side, kind: m.promo.unwrap_or(piece.kind) });
        if piece.kind == Kind::King && (file_of(m.to) - file_of(m.from)).abs() == 2 {
            let rank = rank_of(m.from);
            let (rook_from, rook_to) = if file_of(m.to) == 6 { (7, 5) } else { (0, 3) };
            let rook = n.board[sq(rook_from, rank).expect("kale") as usize].take();
            n.board[sq(rook_to, rank).expect("kale") as usize] = rook;
        }
        // Rok hakları: şah ya da kale oynayınca, ya da köşedeki kale alınınca gider.
        for s in [m.from, m.to] {
            n.castle &= match s {
                4 => !(WK | WQ),
                60 => !(BK | BQ),
                0 => !WQ,
                7 => !WK,
                56 => !BQ,
                63 => !BK,
                _ => !0,
            };
        }
        n.ep = (piece.kind == Kind::Pawn && (rank_of(m.to) - rank_of(m.from)).abs() == 2)
            .then(|| sq(file_of(m.from), (rank_of(m.from) + rank_of(m.to)) / 2).expect("ara kare"));
        n.halfmove = if piece.kind == Kind::Pawn || capture { 0 } else { self.halfmove + 1 };
        n.turn = self.turn.other();
        n
    }

    fn legal_moves(&self) -> Vec<Move> {
        let mut moves = Vec::with_capacity(48);
        self.pseudo_moves(&mut moves);
        moves.retain(|m| !self.play(*m).in_check(self.turn));
        moves
    }

    /// Kimse mat edemez: yalnız şahlar, ya da bir şaha karşı şah ve tek bir hafif taş.
    fn insufficient(&self) -> bool {
        let rest: Vec<Kind> = self.board.iter().flatten().map(|p| p.kind).filter(|k| *k != Kind::King).collect();
        rest.is_empty() || (rest.len() == 1 && matches!(rest[0], Kind::Knight | Kind::Bishop))
    }

    /// Tekrar sayılırken aynı konum: taşlar, sıra, rok hakları, geçerken alma.
    fn key(&self) -> ([Option<Piece>; 64], Side, u8, Option<Sq>) {
        (self.board, self.turn, self.castle, self.ep)
    }
}

/// Bir konumdan `depth` hamle derinliğe kadar oynanabilecek hamle dizilerinin sayısı.
fn perft(pos: &Pos, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = pos.legal_moves();
    if depth == 1 {
        return moves.len() as u64;
    }
    moves.iter().map(|m| perft(&pos.play(*m), depth - 1)).sum()
}

// =============================================================================================
// Arama motoru
// =============================================================================================

const MATE: i32 = 100_000;

/// Merkeze yakınlık: 0 kenarda, 3 tam ortada.
fn centrality(s: Sq) -> i32 {
    let (f, r) = (file_of(s), rank_of(s));
    3 - ((2 * f - 7).abs().max((2 * r - 7).abs()) - 1) / 2
}

/// Sırası gelen tarafın gözünden: malzeme, piyonların ilerleyişi, hafif taşların merkeze yakınlığı,
/// oyun ortasında şahın köşede kalması.
fn evaluate(pos: &Pos) -> i32 {
    let mut score = 0;
    for s in 0..64u8 {
        let Some(p) = pos.board[s as usize] else { continue };
        let advance = if p.side == Side::White { rank_of(s) } else { 7 - rank_of(s) };
        let place = match p.kind {
            Kind::Pawn => advance * 6 + if (2..=5).contains(&file_of(s)) { centrality(s) * 6 } else { 0 },
            Kind::Knight => centrality(s) * 10 - 10,
            Kind::Bishop => centrality(s) * 5,
            Kind::Rook => if advance == 6 { 20 } else { 0 },
            Kind::Queen => centrality(s) * 2,
            Kind::King => -centrality(s) * 8 - if advance > 1 { 20 } else { 0 },
        };
        let v = p.kind.value() + place;
        score += if p.side == pos.turn { v } else { -v };
    }
    score
}

/// Alışlar önce, en değerli kurban en ucuz saldırganla (MVV-LVA); terfiler de önde.
fn order(pos: &Pos, moves: &mut [Move]) {
    let gain = |m: &Move| -> i32 {
        let victim = pos.board[m.to as usize].map_or(if pos.ep == Some(m.to) { 100 } else { 0 }, |p| p.kind.value());
        let attacker = pos.board[m.from as usize].map_or(0, |p| p.kind.value());
        let promo = m.promo.map_or(0, Kind::value);
        if victim > 0 || promo > 0 { victim * 10 - attacker / 10 + promo } else { 0 }
    };
    moves.sort_by_key(|m| std::cmp::Reverse(gain(m)));
}

struct Search {
    nodes: u64,
}

impl Search {
    /// Yalnız alışlar, ta ki konum sakinleşene kadar: derinliğin sınırında yarım bir alış
    /// değişimini kazanç sanmasın diye.
    fn quiesce(&mut self, pos: &Pos, mut alpha: i32, beta: i32) -> i32 {
        self.nodes += 1;
        let stand = evaluate(pos);
        if stand >= beta {
            return beta;
        }
        alpha = alpha.max(stand);
        let mut moves: Vec<Move> = pos
            .legal_moves()
            .into_iter()
            .filter(|m| pos.board[m.to as usize].is_some() || m.promo.is_some() || pos.ep == Some(m.to))
            .collect();
        order(pos, &mut moves);
        for m in moves {
            let score = -self.quiesce(&pos.play(m), -beta, -alpha);
            if score >= beta {
                return beta;
            }
            alpha = alpha.max(score);
        }
        alpha
    }

    fn negamax(&mut self, pos: &Pos, depth: u32, ply: i32, mut alpha: i32, beta: i32) -> i32 {
        if pos.halfmove >= 100 || pos.insufficient() {
            return 0;
        }
        let mut moves = pos.legal_moves();
        if moves.is_empty() {
            // Mat (daha yakın olanı daha kötü) ya da pat.
            return if pos.in_check(pos.turn) { -MATE + ply } else { 0 };
        }
        if depth == 0 {
            return self.quiesce(pos, alpha, beta);
        }
        self.nodes += 1;
        order(pos, &mut moves);
        for m in moves {
            let score = -self.negamax(&pos.play(m), depth - 1, ply + 1, -beta, -alpha);
            if score >= beta {
                return beta;
            }
            alpha = alpha.max(score);
        }
        alpha
    }

    /// En iyi hamle ve değeri. Aynı konum her zaman aynı hamleyi verir.
    fn best(pos: &Pos, depth: u32) -> Option<(Move, i32, u64)> {
        let mut search = Search { nodes: 0 };
        let mut moves = pos.legal_moves();
        order(pos, &mut moves);
        let mut best: Option<(Move, i32)> = None;
        let (mut alpha, beta) = (-MATE - 1, MATE + 1);
        for m in moves {
            let score = -search.negamax(&pos.play(m), depth.saturating_sub(1), 1, -beta, -alpha);
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((m, score));
            }
            alpha = alpha.max(score);
        }
        best.map(|(m, s)| (m, s, search.nodes))
    }
}

// =============================================================================================
// Oyunun sonu
// =============================================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Playing,
    Mate(Side),
    Stalemate,
    FiftyMoves,
    Repetition,
    Insufficient,
}

fn outcome(pos: &Pos, history: &[([Option<Piece>; 64], Side, u8, Option<Sq>)]) -> Outcome {
    if pos.legal_moves().is_empty() {
        return if pos.in_check(pos.turn) { Outcome::Mate(pos.turn.other()) } else { Outcome::Stalemate };
    }
    if pos.halfmove >= 100 {
        return Outcome::FiftyMoves;
    }
    if pos.insufficient() {
        return Outcome::Insufficient;
    }
    let key = pos.key();
    if history.iter().filter(|k| **k == key).count() >= 3 {
        return Outcome::Repetition;
    }
    Outcome::Playing
}

// =============================================================================================
// Sahne
// =============================================================================================

/// Bir taşın sahnedeki ebeveyni.
#[derive(Clone, Copy)]
struct PieceMark;
gizmo::core::impl_component!(PieceMark);

/// Seçimi ve son hamleyi gösteren kare işaretleri.
#[derive(Clone, Copy)]
struct Highlight;
gizmo::core::impl_component!(Highlight);

/// Alınmış, küçülüp gidecek bir taş.
#[derive(Clone, Copy)]
struct Dying;
gizmo::core::impl_component!(Dying);

struct Game {
    pos: Pos,
    /// Her konumun anahtarı ve geri alma için kendisi, oynandığı sırayla (şimdiki dahil).
    history: Vec<Pos>,
    pieces: [Option<Entity>; 64],
    selected: Option<Sq>,
    last: Option<Move>,
    outcome: Outcome,
    /// Motorun oynadığı taraflar.
    engine: [bool; 2],
    depth: u32,
    /// Motor, bir hamleden sonra taşların yerine oturmasını bu kadar bekliyor.
    think_in: f32,
    note: String,
    hud: Entity,
    banner: Entity,
    plies: u32,
}

fn side_index(s: Side) -> usize {
    match s {
        Side::White => 0,
        Side::Black => 1,
    }
}

/// Karenin dünyadaki yeri: beyaz yakında (+Z), a sütunu solda (−X).
fn square_pos(s: Sq) -> Vec3 {
    Vec3::new(file_of(s) as f32 - 3.5, 0.0, 3.5 - rank_of(s) as f32)
}

fn piece_colour(side: Side) -> Vec4 {
    match side {
        // Krem ve kömür: açık karelerden de koyu karelerden de ayrı dursunlar.
        Side::White => Vec4::new(0.80, 0.72, 0.58, 1.0),
        Side::Black => Vec4::new(0.07, 0.065, 0.07, 1.0),
    }
}

/// Bir taşın parçaları: (primitif, yer, ölçek). Hepsi ebeveynin uzayında, tabanı y = 0.
fn piece_parts(kind: Kind) -> Vec<(&'static str, Vec3, Vec3)> {
    let base = ("cylinder", Vec3::new(0.0, 0.06, 0.0), Vec3::new(0.32, 0.06, 0.32));
    match kind {
        Kind::Pawn => vec![
            base,
            ("cylinder", Vec3::new(0.0, 0.22, 0.0), Vec3::new(0.14, 0.12, 0.14)),
            ("sphere", Vec3::new(0.0, 0.44, 0.0), Vec3::splat(0.15)),
        ],
        Kind::Rook => vec![
            base,
            ("cylinder", Vec3::new(0.0, 0.34, 0.0), Vec3::new(0.22, 0.24, 0.22)),
            ("standard_cube", Vec3::new(0.0, 0.64, 0.0), Vec3::new(0.13, 0.035, 0.13)),
        ],
        Kind::Knight => vec![
            base,
            ("cylinder", Vec3::new(0.0, 0.26, 0.0), Vec3::new(0.18, 0.16, 0.18)),
            ("standard_cube", Vec3::new(0.0, 0.56, 0.06), Vec3::new(0.05, 0.1, 0.1)),
            ("standard_cube", Vec3::new(0.0, 0.66, -0.14), Vec3::new(0.045, 0.04, 0.08)),
        ],
        Kind::Bishop => vec![
            base,
            ("cone", Vec3::new(0.0, 0.42, 0.0), Vec3::new(0.2, 0.3, 0.2)),
            ("sphere", Vec3::new(0.0, 0.66, 0.0), Vec3::splat(0.12)),
            ("sphere", Vec3::new(0.0, 0.82, 0.0), Vec3::splat(0.05)),
        ],
        Kind::Queen => vec![
            base,
            ("cylinder", Vec3::new(0.0, 0.42, 0.0), Vec3::new(0.18, 0.32, 0.18)),
            ("cylinder", Vec3::new(0.0, 0.8, 0.0), Vec3::new(0.24, 0.05, 0.24)),
            ("sphere", Vec3::new(0.0, 0.92, 0.0), Vec3::splat(0.09)),
        ],
        Kind::King => vec![
            base,
            ("cylinder", Vec3::new(0.0, 0.46, 0.0), Vec3::new(0.2, 0.36, 0.2)),
            ("cylinder", Vec3::new(0.0, 0.86, 0.0), Vec3::new(0.24, 0.05, 0.24)),
            ("standard_cube", Vec3::new(0.0, 1.04, 0.0), Vec3::new(0.0175, 0.065, 0.0175)),
            ("standard_cube", Vec3::new(0.0, 1.06, 0.0), Vec3::new(0.05, 0.0175, 0.0175)),
        ],
    }
}

fn spawn_piece(world: &mut World, piece: Piece, at: Sq) -> Entity {
    let mut root = Transform::new(square_pos(at));
    // Atlar karşıya baksın: beyazınki kuzeye, siyahınki güneye.
    if piece.side == Side::Black {
        root = root.with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
    }
    let parent = world.spawn_bundle((root, GlobalTransform::default(), PieceMark));
    let colour = piece_colour(piece.side);
    for (shape, offset, scale) in piece_parts(piece.kind) {
        let part = world.spawn_bundle((
            Transform::new(offset).with_scale(scale),
            GlobalTransform::default(),
            MeshSource(shape.into()),
            // Mat bir cila: parlak bir taş güneşte beyaz bir lekeye dönüşüyordu.
            MaterialDesc::new().with_pbr(colour, 0.6, 0.0),
            MeshRenderer::new(),
        ));
        world.add_child(parent, part);
    }
    parent
}

/// Tahtayı ve taşları kurar. Pencere de GPU da gerekmez: `--test` aynısını kullanır.
fn setup(world: &mut World, font: FontId) -> Game {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 1.8, LightRole::Sun),
    ));
    // Çerçeve ve kareler.
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.16, 0.0)).with_scale(Vec3::new(4.5, 0.12, 4.5)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.22, 0.13, 0.07, 1.0), 0.6, 0.0),
        MeshRenderer::new(),
    ));
    for s in 0..64u8 {
        let light = (file_of(s) + rank_of(s)) % 2 == 1;
        let colour = if light { Vec4::new(0.52, 0.40, 0.26, 1.0) } else { Vec4::new(0.24, 0.14, 0.08, 1.0) };
        world.spawn_bundle((
            Transform::new(square_pos(s) - Vec3::Y * 0.03).with_scale(Vec3::new(0.5, 0.03, 0.5)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(colour, 0.7, 0.0),
            MeshRenderer::new(),
        ));
    }
    let hud = world.spawn_bundle((Text::pinned("", font, 24.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    let banner = world.spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.12), Vec2::ZERO)
        .with_anchor(TextAnchor::Center)
        .with_align(TextAlign::Center),));

    let depth = std::env::var("SATRANC_DERINLIK").ok().and_then(|d| d.parse().ok()).unwrap_or(3u32).clamp(1, 6);
    let both = std::env::var_os("SATRANC_OTO").is_some();
    let mut game = Game {
        pos: Pos::start(),
        history: Vec::new(),
        pieces: [None; 64],
        selected: None,
        last: None,
        outcome: Outcome::Playing,
        engine: [both, true],
        depth,
        think_in: 0.0,
        note: String::new(),
        hud,
        banner,
        plies: 0,
    };
    reset(world, &mut game);
    game
}

fn reset(world: &mut World, game: &mut Game) {
    for e in game.pieces.iter_mut().filter_map(Option::take) {
        world.despawn_recursive(e);
    }
    world.despawn_all_with::<Highlight>();
    game.pos = Pos::start();
    game.history = vec![game.pos];
    for s in 0..64u8 {
        if let Some(p) = game.pos.board[s as usize] {
            game.pieces[s as usize] = Some(spawn_piece(world, p, s));
        }
    }
    game.selected = None;
    game.last = None;
    game.outcome = Outcome::Playing;
    game.plies = 0;
    game.note = "Beyaz oynar.".into();
    game.think_in = 0.4;
}

/// Taşları konuma göre yeniden dizer: geri almadan sonra.
fn rebuild(world: &mut World, game: &mut Game) {
    for e in game.pieces.iter_mut().filter_map(Option::take) {
        world.despawn_recursive(e);
    }
    for s in 0..64u8 {
        if let Some(p) = game.pos.board[s as usize] {
            game.pieces[s as usize] = Some(spawn_piece(world, p, s));
        }
    }
}

/// Hamleyi tahtada ve sahnede oynar.
fn make_move(world: &mut World, game: &mut Game, m: Move) {
    let before = game.pos;
    let piece = before.board[m.from as usize].expect("oynayan taş");
    // Alınan taş: hedefte, ya da geçerken almada hedefin gerisinde.
    let taken_at = if piece.kind == Kind::Pawn && Some(m.to) == before.ep && before.board[m.to as usize].is_none() {
        sq(file_of(m.to), rank_of(m.from))
    } else {
        before.board[m.to as usize].map(|_| m.to)
    };
    if let Some(t) = taken_at {
        if let Some(e) = game.pieces[t as usize].take() {
            world.add_component(e, Tween::to_scale(Vec3::ZERO, 0.25).with_ease(Ease::InQuad).with_delay(0.18));
            world.add_component(e, Dying);
        }
    }
    let mover = game.pieces[m.from as usize].take();
    if let Some(e) = mover {
        if let Some(promo) = m.promo {
            // Terfi: piyon gider, yerine yeni taş kareye süzülür.
            world.despawn_recursive(e);
            let fresh = spawn_piece(world, Piece { side: piece.side, kind: promo }, m.from);
            slide(world, fresh, m.to);
            game.pieces[m.to as usize] = Some(fresh);
        } else {
            slide(world, e, m.to);
            game.pieces[m.to as usize] = Some(e);
        }
    }
    // Rokta kale de yer değiştiriyor.
    if piece.kind == Kind::King && (file_of(m.to) - file_of(m.from)).abs() == 2 {
        let rank = rank_of(m.from);
        let (rf, rt) = if file_of(m.to) == 6 { (7, 5) } else { (0, 3) };
        let (rf, rt) = (sq(rf, rank).expect("kale"), sq(rt, rank).expect("kale"));
        if let Some(rook) = game.pieces[rf as usize].take() {
            slide(world, rook, rt);
            game.pieces[rt as usize] = Some(rook);
        }
    }
    game.pos = before.play(m);
    game.history.push(game.pos);
    game.last = Some(m);
    game.selected = None;
    game.plies += 1;
    let keys: Vec<_> = game.history.iter().map(Pos::key).collect();
    game.outcome = outcome(&game.pos, &keys);
    let check = game.pos.in_check(game.pos.turn);
    game.note = format!(
        "{} {}{}{}",
        if piece.side == Side::White { "Beyaz" } else { "Siyah" },
        name(m.from),
        if taken_at.is_some() { "x" } else { "-" },
        name(m.to)
    ) + if check && game.outcome == Outcome::Playing { " — şah!" } else { "" };
    game.think_in = 0.35;
    refresh_highlights(world, game);
}

/// Taşı kareye kaldırıp indirir.
fn slide(world: &mut World, e: Entity, to: Sq) {
    if let Some(e) = world.live(e) {
        world.add_component(e, Tween::to_position(square_pos(to), 0.3).with_ease(Ease::InOutQuad));
    }
}

/// Son hamle sarı, seçili taş yeşil, onun gidebileceği kareler küçük yeşil diskler, şah çekilen şah kırmızı.
fn refresh_highlights(world: &mut World, game: &Game) {
    world.despawn_all_with::<Highlight>();
    let mark = |world: &mut World, s: Sq, colour: Vec4, size: f32| {
        world.spawn_bundle((
            Transform::new(square_pos(s) + Vec3::Y * 0.005).with_scale(Vec3::new(size, 0.006, size)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_unlit(colour),
            MeshRenderer::new(),
            Highlight,
        ));
    };
    if let Some(m) = game.last {
        for s in [m.from, m.to] {
            mark(world, s, Vec4::new(0.85, 0.75, 0.2, 1.0), 0.48);
        }
    }
    if let Some(sel) = game.selected {
        mark(world, sel, Vec4::new(0.3, 0.8, 0.35, 1.0), 0.48);
        for m in game.pos.legal_moves().into_iter().filter(|m| m.from == sel) {
            mark(world, m.to, Vec4::new(0.25, 0.7, 0.3, 1.0), 0.16);
        }
    }
    if game.pos.in_check(game.pos.turn) {
        if let Some(k) = game.pos.king(game.pos.turn) {
            mark(world, k, Vec4::new(0.9, 0.15, 0.1, 1.0), 0.48);
        }
    }
}

fn animating(world: &World) -> bool {
    world.query::<&Tween>().iter().any(|t| !t.is_finished())
}

fn cursor_square(world: &World) -> Option<Sq> {
    let p = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Y)?;
    sq((p.x + 4.0).floor() as i32, (4.0 - p.z).floor() as i32)
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    if !animating(world) {
        // Alınmış taşlar küçülüp bitti: parçalarıyla birlikte gitsin.
        let dying: Vec<Entity> = world.query::<(Entity, &Dying)>().iter().map(|(e, _)| e).collect();
        for e in dying {
            world.despawn_recursive(e);
        }
    }
    if input.is_key_just_pressed(KeyCode::KeyY as u32) {
        reset(world, &mut game);
    }
    if input.is_key_just_pressed(KeyCode::KeyM as u32) {
        let both = !game.engine[0];
        game.engine = [both, true];
    }
    if input.is_key_just_pressed(KeyCode::KeyU as u32) && game.history.len() >= 3 {
        // Son hamle çifti geri: oyuncunun hamlesi ve motorun cevabı.
        game.history.truncate(game.history.len() - 2);
        game.pos = *game.history.last().expect("başlangıç konumu kalır");
        game.plies = game.plies.saturating_sub(2);
        game.last = None;
        game.selected = None;
        game.outcome = Outcome::Playing;
        game.note = "İki hamle geri alındı.".into();
        rebuild(world, &mut game);
        refresh_highlights(world, &game);
    }

    if game.outcome == Outcome::Playing && !animating(world) {
        let me = game.pos.turn;
        if game.engine[side_index(me)] {
            game.think_in -= dt;
            if game.think_in <= 0.0 {
                if let Some((m, score, nodes)) = Search::best(&game.pos, game.depth) {
                    make_move(world, &mut game, m);
                    game.note += &format!("   (motor: {nodes} konum, değer {:+.2})", f64::from(score) / 100.0);
                }
            }
        } else if input.is_mouse_button_just_pressed(0) {
            if let Some(s) = cursor_square(world) {
                let own = game.pos.board[s as usize].is_some_and(|p| p.side == me);
                if own {
                    game.selected = Some(s);
                    refresh_highlights(world, &game);
                } else if let Some(from) = game.selected {
                    // Terfide R/B/N basılıysa kale/fil/at, yoksa vezir.
                    let wanted = if input.is_key_pressed(KeyCode::KeyR as u32) {
                        Kind::Rook
                    } else if input.is_key_pressed(KeyCode::KeyB as u32) {
                        Kind::Bishop
                    } else if input.is_key_pressed(KeyCode::KeyN as u32) {
                        Kind::Knight
                    } else {
                        Kind::Queen
                    };
                    let legal = game.pos.legal_moves();
                    let chosen = legal.iter().find(|m| m.from == from && m.to == s && m.promo.is_none_or(|k| k == wanted));
                    match chosen {
                        Some(m) => make_move(world, &mut game, *m),
                        None => {
                            game.selected = None;
                            refresh_highlights(world, &game);
                        }
                    }
                }
            }
        }
    }
    draw_hud(world, &game);
    world.insert_resource(game);
}

fn draw_hud(world: &mut World, game: &Game) {
    let turn = if game.pos.turn == Side::White { "Beyaz" } else { "Siyah" };
    let who = |s: Side| if game.engine[side_index(s)] { "motor" } else { "sen" };
    let hud = format!(
        "Hamle {}   Sıra: {turn}   (beyaz: {}, siyah: {}, derinlik {})\n{}\nTıkla: seç ve oyna · U geri al · M motor/sen · Y yeni oyun",
        game.plies / 2 + 1,
        who(Side::White),
        who(Side::Black),
        game.depth,
        game.note
    );
    let banner = match game.outcome {
        Outcome::Playing => String::new(),
        Outcome::Mate(winner) => format!("Mat — {} kazandı", if winner == Side::White { "beyaz" } else { "siyah" }),
        Outcome::Stalemate => "Pat — berabere".into(),
        Outcome::FiftyMoves => "Elli hamle kuralı — berabere".into(),
        Outcome::Repetition => "Üç kez tekrar — berabere".into(),
        Outcome::Insufficient => "Mat edecek taş yok — berabere".into(),
    };
    gizmo::simple::set_text(world, game.hud, &hud);
    gizmo::simple::set_text(world, game.banner, &banner);
}

// =============================================================================================
// Pencere ve sınama
// =============================================================================================

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Satranç", 1280, 900)
        .with_simple_scene(|scene, state| {
            let font = load_font(scene.renderer);
            scene.spawn_camera(state, Vec3::new(0.0, 9.0, 7.5), Vec3::new(0.0, 0.0, 0.6));
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `--test`: kurallar perft ile, motor bir hamlede matla, sahne de motorun kendine karşı oynadığı
/// bir maçla.
fn self_test() -> bool {
    let mut ok = true;
    // Satranç programlamasının bilinen sayıları (chessprogramming.org, "Perft Results").
    let cases = [
        ("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", "başlangıç", vec![20u64, 400, 8_902, 197_281]),
        ("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1", "Kiwipete", vec![48, 2_039, 97_862]),
        ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", "konum 3", vec![14, 191, 2_812, 43_238]),
        ("r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1", "konum 4", vec![6, 264, 9_467]),
    ];
    for (fen, label, expected) in cases {
        let pos = Pos::fen(fen).expect("FEN");
        for (d, want) in expected.iter().enumerate() {
            let got = perft(&pos, d as u32 + 1);
            if got != *want {
                println!("satranc --test: perft {label} derinlik {}: {got}, beklenen {want} → BAŞARISIZ", d + 1);
                ok = false;
            }
        }
    }
    println!("satranc --test: perft (dört konum, 13 derinlik) → {}", if ok { "tamam" } else { "BAŞARISIZ" });

    // Bir hamlede mat: kale arka sıraya.
    let mate_in_one = Pos::fen("6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1").expect("FEN");
    let found = Search::best(&mate_in_one, 3).map(|(m, s, _)| (name(m.from), name(m.to), s));
    let mates = found.as_ref().is_some_and(|(f, t, s)| f == "d1" && t == "d8" && *s >= MATE - 10);
    println!("satranc --test: bir hamlede mat → {found:?} {}", if mates { "tamam" } else { "BAŞARISIZ" });
    ok &= mates;

    // Motor kendine karşı: sahne tahtayla aynı mı kalıyor.
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font());
    game.engine = [true, true];
    game.depth = 2;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let done = |w: &World| w.get_resource::<Game>().is_none_or(|g| g.outcome != Outcome::Playing || g.plies >= 160);
    while run.elapsed < 600.0 && !(done(&run.world) && !animating(&run.world)) {
        run.frame(1.0 / 30.0, update);
    }
    run.frame(1.0 / 30.0, update);
    let game = run.world.get_resource::<Game>().expect("oyun");
    let on_board = game.pos.board.iter().flatten().count();
    let mut placed = 0;
    let mut alive = 0;
    for s in 0..64u8 {
        if let Some(e) = game.pieces[s as usize] {
            alive += 1;
            let at = run.world.query::<&Transform>().get(e).map(|t| t.position);
            if at.is_some_and(|p| (p - square_pos(s)).length() < 0.01) && game.pos.board[s as usize].is_some() {
                placed += 1;
            }
        }
    }
    let marks = run.world.query::<&PieceMark>().iter().count();
    let scene_ok = game.plies >= 20 && alive == on_board && placed == on_board && marks == on_board;
    println!(
        "satranc --test: motor kendine karşı {} yarım hamle, sonuç {:?}; tahtada {on_board} taş, sahnede {marks}, {placed}'i kendi karesinde → {}",
        game.plies,
        game.outcome,
        if scene_ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok && scene_ok
}

fn load_font(renderer: &mut Renderer) -> FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
