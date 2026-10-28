// Listes à fiches (vinyles, Lego, livres…) : des listes dont chaque élément est une fiche aux champs propres,
// créées et remplies à la voix, affichées en mini-cards, gérées depuis la card du HUD ou la page d'aide, et
// retrouvées par le RAG. Voir service.rs (opérations), schema.rs (champs), voice.rs (phrases), index.rs (RAG).

pub mod cards;
pub mod catalog;
pub mod enrich;
pub mod index;
pub mod query;
pub mod schema;
pub mod service;
pub mod voice;

pub use service::Collections;
