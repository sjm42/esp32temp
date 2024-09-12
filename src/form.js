var postCfgDataAsJson = async ({
                                   url, formData
                               }) => {
    const formObj = Object.fromEntries(formData.entries());
    formObj.port = parseInt(formObj.port);
    formObj.retries = parseInt(formObj.retries);
    formObj.delay = parseInt(formObj.delay);
    formObj.v4dhcp = (formObj.v4dhcp === "on");
    formObj.v4mask = parseInt(formObj.v4mask);
    formObj.mqtt_enable = (formObj.mqtt_enable === "on");
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
    o.innerHTML = "<p>Uptime: " + json.uptime + " </p>";
}

function onLoad() {
    // update_uptime();
    setInterval(update_uptime, 10e3);
}
