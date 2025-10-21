// form.js for esp32temp

document.addEventListener("DOMContentLoaded", function () {
    document.querySelector("form[name='esp32cfg']")
        .addEventListener("submit", handleCfgSubmit);
    setInterval(update_uptime, 10e3);
    setInterval(update_temperatures, 60e3);
});

async function update_uptime() {
    let o = document.getElementById("uptime");
    // o.innerHTML = "<p>Uptime: -updating-</p>";
    const url = "/uptime";
    const response = await fetch(url);
    const json = JSON.parse(await response.text());
    o.innerHTML = `<p>Uptime: ${json.uptime} (${json.uptime_s})</p>`;
}

async function update_temperatures() {
    let o = document.getElementById("temperatures");
    // o.innerHTML = "<p>-updating-</p>";
    const url = "/temp";
    const response = await fetch(url);
    const json = JSON.parse(await response.text());
    let rows = "<tr><th>iopin</th><th>sensor</th><th>value</th></tr>\n";
    json.temperatures.forEach((temp, index) => {
        rows += `<tr><td><i>${temp.iopin}</i></td><td>${temp.sensor}</td><td><b>${temp.value}</b></td></tr>\n`;
    });
    o.innerHTML = `<p>Last update: <b>${json.last_update}</b><br>\n<table>\n${rows}</table></p>\n`;
}

let handleCfgSubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const url = form.action;

    try {
        const formData = new FormData(form);
        const responseData = await postCfgDataAsJson({url, formData});
        console.log({
            responseData
        });
    } catch (error) {
        console.error(error);
    }
}

let postCfgDataAsJson = async ({url, formData}) => {
    const formObj = Object.fromEntries(formData.entries());
    // convert integers
    formObj.port = parseInt(formObj.port);
    formObj.v4mask = parseInt(formObj.v4mask);
    formObj.retries = parseInt(formObj.retries);
    formObj.delay = parseInt(formObj.delay);
    // convert booleans
    formObj.wifi_wpa2ent = (formObj.wifi_wpa2ent === "on");
    formObj.v4dhcp = (formObj.v4dhcp === "on");
    formObj.mqtt_enable = (formObj.mqtt_enable === "on");
    // serialize to JSON
    const formDataJsonString = JSON.stringify(formObj);

    const fetchOptions = {
        method: "POST", mode: 'cors', keepalive: false, headers: {
            'Accept': 'application/json', 'Content-Type': 'application/json',
        }, body: formDataJsonString,
    };
    const response = await fetch(url, fetchOptions);

    if (!response.ok) {
        const errorMessage = await response.text();
        throw new Error(errorMessage);
    }

    return response.json();
}
// EOF
