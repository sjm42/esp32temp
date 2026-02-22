// form.js for esp32temp

document.addEventListener("DOMContentLoaded", function () {
    bindForm("esp32cfg", handleCfgSubmit);
    bindForm("esp32fw", handleFwSubmit);
    initUptime();
    initTemperatures();
});

function bindForm(name, handler) {
    const form = document.querySelector(`form[name='${name}']`);
    if (!form) return;
    ensureStatusNode(form);
    form.addEventListener("submit", handler);
}

function ensureStatusNode(form) {
    let status = form.querySelector(".form-status");
    if (status) return status;

    status = document.createElement("div");
    status.className = "form-status";
    status.setAttribute("role", "status");
    status.setAttribute("aria-live", "polite");
    status.hidden = true;
    form.appendChild(status);
    return status;
}

function setFormStatus(form, kind, message) {
    const status = ensureStatusNode(form);
    status.hidden = !message;
    status.className = `form-status${kind ? ` is-${kind}` : ""}`;
    status.textContent = message || "";
}

function setFormBusy(form, busy, busyLabel) {
    const submit = form.querySelector("input[type='submit']");
    if (!submit) return;

    if (busy) {
        if (!submit.dataset.label) submit.dataset.label = submit.value;
        submit.disabled = true;
        submit.value = busyLabel || "Working...";
    } else {
        submit.disabled = false;
        if (submit.dataset.label) submit.value = submit.dataset.label;
    }
}

async function fetchPayloadOrError(url, options) {
    const response = await fetch(url, options);
    const contentType = response.headers.get("content-type") || "";

    let payload;
    if (contentType.includes("application/json")) {
        payload = await response.json();
    } else {
        const text = await response.text();
        payload = {message: (text || "").trim()};
    }

    if (!response.ok) {
        throw new Error(payload.message || `Request failed (${response.status})`);
    }
    return payload;
}

async function updateUptime() {
    const node = document.getElementById("uptime");
    if (!node) return;

    try {
        const response = await fetch("/uptime");
        const json = await response.json();
        node.textContent = `Uptime: ${json.uptime} (${json.uptime_s})`;
    } catch (_error) {
        node.textContent = "Uptime unavailable";
    }
}

function initUptime() {
    if (!document.getElementById("uptime")) return;
    updateUptime();
    window.setInterval(updateUptime, 10e3);
}

async function updateTemperatures() {
    const node = document.getElementById("temperatures");
    if (!node) return;

    try {
        const response = await fetch("/temp");
        const json = await response.json();
        let rows = "<tr><th>IO pin</th><th>Sensor</th><th>Value (C)</th></tr>\n";
        json.temperatures.forEach((temp) => {
            rows += `<tr><td><code>${temp.iopin}</code></td><td>${temp.sensor}</td><td class="temperature-value">${temp.value}</td></tr>\n`;
        });
        node.innerHTML =
            `<div class="temperature-meta">Last update: <b>${json.last_update}</b></div>` +
            `<table>${rows}</table>`;
    } catch (_error) {
        node.textContent = "Temperature data unavailable";
    }
}

function initTemperatures() {
    if (!document.getElementById("temperatures")) return;
    updateTemperatures();
    window.setInterval(updateTemperatures, 60e3);
}

const handleCfgSubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const url = form.action;

    setFormBusy(form, true, "Saving...");
    setFormStatus(form, "busy", "Saving config...");
    try {
        const formData = new FormData(form);
        const responseData = await postCfgDataAsJson({url, formData});
        setFormStatus(form, "ok", responseData.message || "Config saved, device will reboot");
    } catch (error) {
        console.error(error);
        setFormStatus(form, "error", error.message || "Config save failed");
    } finally {
        setFormBusy(form, false);
    }
};

const handleFwSubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const url = form.action;

    if (!window.confirm("Start firmware update now? The device will reboot if the update succeeds.")) {
        return;
    }

    setFormBusy(form, true, "Updating...");
    setFormStatus(form, "busy", "Downloading and flashing firmware...");
    try {
        const formData = new FormData(form);
        const responseData = await postFwForm({url, formData});
        setFormStatus(form, "ok", responseData.message || "Firmware update started");
    } catch (error) {
        console.error(error);
        setFormStatus(form, "error", error.message || "Firmware update failed");
    } finally {
        setFormBusy(form, false);
    }
};

const postCfgDataAsJson = async ({url, formData}) => {
    const formObj = Object.fromEntries(formData.entries());
    formObj.port = parseInt(formObj.port, 10);
    formObj.v4mask = parseInt(formObj.v4mask, 10);
    formObj.retries = parseInt(formObj.retries, 10);
    formObj.delay = parseInt(formObj.delay, 10);
    formObj.wifi_wpa2ent = (formObj.wifi_wpa2ent === "on");
    formObj.v4dhcp = (formObj.v4dhcp === "on");
    formObj.mqtt_enable = (formObj.mqtt_enable === "on");

    return fetchPayloadOrError(url, {
        method: "POST",
        mode: "cors",
        keepalive: false,
        headers: {"Accept": "application/json", "Content-Type": "application/json"},
        body: JSON.stringify(formObj)
    });
};

const postFwForm = async ({url, formData}) => {
    const params = new URLSearchParams(formData);
    return fetchPayloadOrError(url, {
        method: "POST",
        body: params
    });
};

// EOF
