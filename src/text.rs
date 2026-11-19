//! The texts of the plugin, in English and French.

pub fn t(french: bool, key: &str) -> &'static str {
    let (en, fr) = match key {
        "title" => ("Local history", "Historique local"),
        "pick" => (
            "Open a file, then Local History: Show (or the menu of a file or folder)",
            "Ouvrez un fichier, puis Historique local : afficher (ou le menu d'un fichier ou d'un dossier)",
        ),
        "search" => ("Search in the revisions", "Chercher dans les révisions"),
        "empty" => ("Nothing kept yet", "Rien n'est encore gardé"),
        "off" => (
            "The local history is off (Preferences › Plugins › Local History)",
            "L'historique local est désactivé (Préférences › Plugins › Historique local)",
        ),
        "compare" => ("Compare with now", "Comparer avec maintenant"),
        "revert" => ("Revert", "Restaurer"),
        "revert_ask" => (
            "Put the files back as they were then? What they hold now is kept in the history first.",
            "Remettre les fichiers comme ils étaient ? Ce qu'ils contiennent maintenant est d'abord gardé dans l'historique.",
        ),
        "reverted" => ("Put back", "Restauré"),
        "patch" => ("Save as patch…", "Enregistrer en patch…"),
        "label" => ("Put label", "Poser un label"),
        "label_hint" => ("Label name", "Nom du label"),
        "labelled" => ("Label put", "Label posé"),
        "auto_commit" => ("Commit", "Commit"),
        "now" => ("now", "maintenant"),
        "change" => ("Change at line", "Changement à la ligne"),
        "revert_change" => ("Revert this change", "Restaurer ce changement"),
        "same" => ("The texts are the same", "Les textes sont identiques"),
        "gone" => (
            "That revision is no longer kept",
            "Cette révision n'est plus gardée",
        ),
        "days" => ("Keep the history for", "Garder l'historique pendant"),
        "not_saved" => (
            "The settings could not be saved",
            "Les réglages n'ont pas pu être enregistrés",
        ),
        _ => ("", ""),
    };
    if french {
        fr
    } else {
        en
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_text_exists_in_both_languages() {
        for key in [
            "title",
            "pick",
            "search",
            "empty",
            "off",
            "compare",
            "revert",
            "revert_ask",
            "reverted",
            "patch",
            "change",
            "revert_change",
            "label",
            "label_hint",
            "labelled",
            "auto_commit",
            "now",
            "same",
            "gone",
            "days",
            "not_saved",
        ] {
            assert!(
                !super::t(false, key).is_empty() && !super::t(true, key).is_empty(),
                "{key}"
            );
        }
    }
}
