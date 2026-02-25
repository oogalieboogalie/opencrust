import { createGoogleIntegrationController } from '/assets/webchat/integrations/google.js';

const SERVICE_META = {
  calendar: { title: "Google Calendar", scopes: "calendar.readonly, calendar.events" },
  gmail: { title: "Gmail", scopes: "gmail.readonly, gmail.send" },
  drive: { title: "Google Drive", scopes: "drive.readonly" },
  sheets: { title: "Google Sheets", scopes: "spreadsheets" },
  contacts: { title: "Google Contacts", scopes: "contacts.readonly" },
};

export function initIntegrationsView({ onSystemMessage, onErrorMessage }) {
  const googleIntegrationStatusEl = document.getElementById("google-integration-status");
  const googleIntegrationMetaEl = document.getElementById("google-integration-meta");
  const googleIntegrationToggleBtn = document.getElementById("google-integration-toggle");
  const googleClientIdInputEl = document.getElementById("google-client-id-input");
  const googleClientSecretInputEl = document.getElementById("google-client-secret-input");
  const googleRedirectUriInputEl = document.getElementById("google-redirect-uri-input");
  const googleIntegrationSaveBtn = document.getElementById("google-integration-save");
  const googleIntegrationFeedbackEl = document.getElementById("google-integration-feedback");
  const googleVaultStatusEl = document.getElementById("google-vault-status");

  const modal = document.getElementById("integration-modal");
  const modalCloseBtn = document.getElementById("modal-close");
  const modalTitle = document.getElementById("modal-title");
  const modalConnectionStatus = document.getElementById("modal-connection-status");
  const modalScopesEl = modal?.querySelector(".modal-help code");

  // Google integration controller (shared across all Google services)
  const googleIntegration = createGoogleIntegrationController({
    statusEl: googleIntegrationStatusEl,
    metaEl: googleIntegrationMetaEl,
    toggleBtn: googleIntegrationToggleBtn,
    clientIdInputEl: googleClientIdInputEl,
    clientSecretInputEl: googleClientSecretInputEl,
    redirectUriInputEl: googleRedirectUriInputEl,
    saveBtn: googleIntegrationSaveBtn,
    vaultStatusEl: googleVaultStatusEl,
    feedbackEl: googleIntegrationFeedbackEl,
    onSystemMessage,
    onErrorMessage,
  });

  googleIntegration.attach();

  // ── Modal open/close ──

  function openModal(service) {
    if (!modal) return;
    const meta = SERVICE_META[service] || { title: service, scopes: "—" };
    if (modalTitle) modalTitle.textContent = meta.title;
    if (modalScopesEl) modalScopesEl.textContent = meta.scopes;

    // Sync the card's status pill into the modal
    const cardEl = document.querySelector(`.conn-card[data-service="${service}"]`);
    const cardStatus = cardEl?.querySelector("[data-status]");
    if (modalConnectionStatus && cardStatus) {
      modalConnectionStatus.textContent = cardStatus.textContent;
      modalConnectionStatus.className = cardStatus.className;
    }

    modal.classList.add("is-open");
    modal.setAttribute("aria-hidden", "false");
  }

  function closeModal() {
    if (!modal) return;
    modal.classList.remove("is-open");
    modal.setAttribute("aria-hidden", "true");
  }

  // Attach click to all Configure buttons
  document.querySelectorAll(".conn-configure").forEach((btn) => {
    btn.addEventListener("click", () => {
      const service = btn.getAttribute("data-service");
      openModal(service);
    });
  });

  if (modalCloseBtn) {
    modalCloseBtn.addEventListener("click", closeModal);
  }

  // Close on backdrop click
  if (modal) {
    modal.addEventListener("click", (e) => {
      if (e.target === modal) closeModal();
    });
  }

  // Close on Escape
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && modal?.classList.contains("is-open")) {
      closeModal();
    }
  });

  return {
    load: () => googleIntegration.load(),
  };
}
