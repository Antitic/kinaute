# ADR 0002 — Rendu CPU en premier

**Statut** : accepté.

**Contexte** : la machine de référence est un PC de 2008 sans GPU moderne ; démarrage instantané et faible mémoire exigés.

**Décision** : tout le rendu passe par tiny-skia (rasteriseur vectoriel) et cosmic-text (mise en forme). Pas de dépendance GPU.

**Alternatives écartées** : wgpu/vello (exigent Vulkan/Metal/DX12 ou au minimum un GL récent, démarrage et mémoire plus lourds) ;
skia-safe (binaire et compilation très lourds) ; rendu maison (coût disproportionné).

**Techniques pour tenir la fluidité** :
- pendant un glisser, les objets fixes sont mis en cache en deux couches (dessous / dessus) et seul l'objet déplacé est redessiné ;
- la composition est limitée à la zone de l'objet, la diapositive opaque est copiée ligne par ligne ;
- les ombres floues sont mises en cache (elles ne dépendent que de la forme) ;
- les vignettes sont rendues en tâche de fond, une à la fois, pour ne jamais bloquer la saisie ;
- les images sont décodées à la demande et réduites à la taille d'affichage.

**Conséquences** : les effets très lourds (flous larges, grandes images haute résolution) restent le point faible ; un GPU facultatif
pourra s'ajouter plus tard sans changer le modèle.
