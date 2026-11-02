# ADR 0003 — Slint pour l'interface

**Statut** : accepté, à surveiller.

**Décision** : interface en Slint avec le moteur de rendu logiciel et le backend winit. Le canevas est une image produite par notre
moteur (pas de widget de dessin spécifique) : un seul chemin de rendu pour l'édition, le diaporama, les vignettes et l'export.

**Pourquoi** : rendu logiciel natif, licence compatible GPL-3, accessibilité (AccessKit), widgets de saisie avec IME, empreinte faible.

**Alternatives** : egui (immédiat, texte et accessibilité moins matures), iced (dépend de wgpu/tiny-skia selon le mode, plus lourd),
interface maison winit (coût élevé, IME et accessibilité à refaire).

**Mesuré** (un cœur) : fenêtre en 66–227 ms, 37–75 Mo, binaire 21 Mo.

**Limites actuelles** : la saisie de texte se fait dans l'inspecteur (widget Slint, donc IME correct) et non directement sur le canevas ;
pas de menu natif ; pas de sélecteur de fichiers intégré (zenity/kdialog si présents, sinon une saisie de chemin).
