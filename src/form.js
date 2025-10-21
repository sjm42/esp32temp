// form.js for esp32temp
var postCfgDataAsJson = async ({
                                   url, formData
                               }) => {
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

var handleCfgSubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const url = form.action;

    try {
        formData = new FormData(form);
        const responseData = await postCfgDataAsJson({
            url, formData
        });
        console.log({
            responseData
        });
    } catch (error) {
        console.error(error);
    }
}

document.addEventListener("DOMContentLoaded", function () {
    document.querySelector("form[name='esp32cfg']")
        .addEventListener("submit", handleCfgSubmit);
});

async function update_uptime() {
    var o = document.getElementById("uptime");
    o.innerHTML = "Updating...";
    var url = "/uptime";
    const response = await fetch(url);
    const json = JSON.parse(await response.text());
    o.innerHTML = `<p>Uptime: ${json.uptime} (${json.uptime_s}) </p>`;
}

function onLoad() {
    // update_uptime();
    setInterval(update_uptime, 10e3);
}

// EOF
