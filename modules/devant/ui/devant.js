// Module Devant : la card « Devant » (applications ouvertes en liste verticale, mini-cards, applications retirées).
// Le noyau (orch/devant.rs) envoie l'événement « widget » kind devant et reçoit les actions (devantAction).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, gripHandle, closeButton, openOnClick } = K;

  // --- Card « Devant » : liste verticale des applications avec mini-cards et sous-menu des retirées ---
  function renderDevant(ev) {
    const key = `tool:${ev.title || 'Devant'}`;
    const ttl = ev.pinned || ev.ttl === 0 ? 0 : (ev.ttl || 120000);
    const isShowingExcluded = Boolean(ev.showExcluded);
    const items = Array.isArray(ev.items) ? ev.items : [];
    const excluded = Array.isArray(ev.excluded) ? ev.excluded : [];

    const el = h('div', { class: 'widget w-tool w-card-list w-card-devant' });

    // Bouton de bascule vers / depuis le sous-menu des applications retirées
    const toggleSubMenuBtn = h('button', {
      class: `w-devant-menu-btn${isShowingExcluded ? ' active' : ''}`,
      title: isShowingExcluded ? 'Retourner à la liste des applications ouvertes' : 'Gérer les applications retirées de la liste',
      onclick: (e) => {
        e.stopPropagation();
        if (bridge && bridge.devantAction) {
          bridge.devantAction('toggle_view');
        } else {
          ev.showExcluded = !ev.showExcluded;
          renderDevant(ev);
        }
      }
    }, isShowingExcluded ? '←' : `👁‍🗨${excluded.length ? ` ${excluded.length}` : ''}`);

    const headEl = h('div', { class: 'w-head' },
      gripHandle(),
      h('span', { class: 'w-icon' }, '🗔'),
      h('span', { class: 'w-title' }, 'Devant'),
      toggleSubMenuBtn,
      closeButton(ev, key)
    );

    el.append(headEl);

    if (isShowingExcluded) {
      // --- Vue Sous-menu des applications retirées ---
      const subHeader = h('div', { class: 'w-devant-sub-header' },
        h('div', { class: 'w-devant-sub-title' }, `Masquées (${excluded.length})`),
        excluded.some((x) => x.type === 'temp') ? h('button', {
          class: 'w-devant-btn-clear-temp',
          title: 'Restaurer toutes les applications retirées temporairement',
          onclick: (e) => {
            e.stopPropagation();
            if (bridge && bridge.devantAction) bridge.devantAction('restore_all');
          }
        }, '↺ Tout') : null
      );

      const subListEl = h('div', { class: 'w-devant-excluded-list' });

      if (excluded.length === 0) {
        subListEl.append(h('div', { class: 'w-empty' }, 'Aucune application retirée.'));
      } else {
        for (const ex of excluded) {
          const itemEl = h('div', { class: 'w-devant-excluded-item' },
            h('span', { class: 'w-devant-icon' }, ex.icon || '🗔'),
            h('div', { class: 'w-devant-ex-info' },
              h('span', { class: 'w-devant-name' }, ex.app || ex.key)
            ),
            h('div', { class: 'w-devant-ex-actions' },
              h('button', {
                class: 'w-devant-btn-toggle-type',
                title: ex.type === 'perm' ? 'Passer en retrait temporaire' : 'Passer en exclusion définitive',
                onclick: (e) => {
                  e.stopPropagation();
                  if (bridge && bridge.devantAction) bridge.devantAction('toggle_type', ex.key || ex.app);
                }
              }, ex.type === 'perm' ? '⏱' : '🚫'),
              h('button', {
                class: 'w-devant-btn-restore',
                title: `Restaurer ${ex.app || ex.key} dans la liste Devant`,
                onclick: (e) => {
                  e.stopPropagation();
                  if (bridge && bridge.devantAction) bridge.devantAction('restore', ex.key || ex.app);
                }
              }, '↺')
            )
          );
          subListEl.append(itemEl);
        }
      }

      const backBtn = h('button', {
        class: 'w-devant-back-btn',
        onclick: (e) => {
          e.stopPropagation();
          if (bridge && bridge.devantAction) {
            bridge.devantAction('show_active');
          } else {
            ev.showExcluded = false;
            renderDevant(ev);
          }
        }
      }, '← Retour');

      el.append(subHeader, subListEl, backBtn);

    } else {
      // --- Vue principale : Liste verticale des applications ouvertes (Mini-cards) ---
      const listEl = h('div', { class: 'w-devant-list' });

      if (items.length === 0) {
        listEl.append(h('div', { class: 'w-empty' }, ev.empty || 'Aucune application ouverte.'));
      } else {
        for (const item of items) {
          const miniCard = h('div', {
            class: `w-devant-mini-card${item.fg ? ' is-foreground' : ''}${item.min ? ' is-minimized' : ''}`,
            role: 'button',
            tabindex: '0',
            title: item.min ? `Clic : agrandir ${item.app} et l'amener au premier plan` : `Clic : amener ${item.app} au premier plan`,
            onclick: (e) => {
              if (e.target.closest('.w-devant-btn-remove') || e.target.closest('.w-devant-btn-screen')) return;
              miniCard.classList.add('w-devant-clicked');
              setTimeout(() => miniCard.classList.remove('w-devant-clicked'), 400);

              // Réorganisation visuelle immédiate en haut de liste
              if (listEl.firstChild !== miniCard) {
                listEl.prepend(miniCard);
              }
              listEl.querySelectorAll('.w-devant-mini-card').forEach((c) => c.classList.remove('is-foreground'));
              miniCard.classList.add('is-foreground');
              miniCard.classList.remove('is-minimized');

              if (document.activeElement && typeof document.activeElement.blur === 'function') {
                document.activeElement.blur();
              }
              const b = window.jarvisBridge || bridge;
              if (b && b.desktopAction) {
                b.desktopAction(item.hwnd, item.min ? 'agrandir' : 'afficher');
              }
            },
            // Double-clic : fenêtre restaurée, centrée sur son écran et devant toutes les autres (entièrement visible).
            ondblclick: (e) => {
              if (e.target.closest('.w-devant-btn-remove') || e.target.closest('.w-devant-btn-screen')) return;
              const b = window.jarvisBridge || bridge;
              if (b && b.desktopAction) b.desktopAction(item.hwnd, 'centrer');
            }
          });

          // Bouton interactif pour basculer la fenêtre d'un écran à l'autre
          const curMon = item.monitor || 1;
          const otherMon = curMon === 1 ? 2 : 1;
          const screenBtn = h('button', {
            class: 'w-devant-btn-screen',
            title: `Déplacer vers l'écran ${otherMon}`,
            onclick: (e) => {
              e.stopPropagation();
              const b = window.jarvisBridge || bridge;
              if (b && b.desktopAction) {
                b.desktopAction(item.hwnd, 'deplacer');
              }
            }
          }, `Écr. ${curMon}`);

          // Badge ou bouton d'état (Devant ou Réduite)
          let statusBadge = null;
          if (item.fg) {
            statusBadge = h('span', { class: 'w-devant-badge fg', title: 'Au premier plan' }, '● Devant');
          } else if (item.min) {
            statusBadge = h('button', {
              class: 'w-devant-badge min clickable',
              title: `Agrandir ${item.app} et passer au premier plan`,
              onclick: (e) => {
                e.stopPropagation();
                const b = window.jarvisBridge || bridge;
                if (b && b.desktopAction) {
                  b.desktopAction(item.hwnd, 'agrandir');
                }
              }
            }, 'Réduite');
          }

          // Ligne du haut : icône, nom d'appli, bouton écran, statut, boutons retrait
          const topEl = h('div', { class: 'w-devant-top' },
            h('span', { class: 'w-devant-icon' }, item.icon || '🗔'),
            h('span', { class: 'w-devant-name', title: item.app }, item.app),
            screenBtn,
            statusBadge,
            h('div', { class: 'w-devant-actions' },
              h('button', {
                class: 'w-devant-btn-remove temp',
                title: `Retirer ${item.app} temporairement de la liste`,
                onclick: (e) => {
                  e.stopPropagation();
                  const b = window.jarvisBridge || bridge;
                  if (b && b.devantAction) {
                    b.devantAction('remove_temp', item.appKey || item.app);
                  }
                }
              }, '⊖'),
              h('button', {
                class: 'w-devant-btn-remove perm',
                title: `Exclure ${item.app} définitivement de la liste`,
                onclick: (e) => {
                  e.stopPropagation();
                  const b = window.jarvisBridge || bridge;
                  if (b && b.devantAction) {
                    b.devantAction('remove_perm', item.appKey || item.app);
                  }
                }
              }, '🚫')
            )
          );

          // Ligne du bas : titre de la fenêtre / page active
          const windowTitle = item.page || item.title || '(sans titre)';
          const bodyEl = h('div', { class: 'w-devant-title', title: item.title || windowTitle }, windowTitle);

          miniCard.append(topEl, bodyEl);
          listEl.append(miniCard);
        }
      }

      el.append(listEl);
    }

    if (ev.footer) {
      el.append(h('div', { class: 'w-rows-foot' }, ev.footer));
    }

    openOnClick(el, ev);
    place(key, el, 'devant', ttl);
  }

  K.register('devant', renderDevant);
})();
