#include "shim.h"

WelleIoBridge::WelleIoBridge() : counter_(0), receiver_(nullptr) {}

int32_t WelleIoBridge::ping() const {
    return counter_ + 1;
}

void WelleIoBridge::setReceiver(RadioReceiver* receiver) {
    receiver_ = receiver;
}

void WelleIoBridge::setSnrCallback(rust::Fn<void(float)> callback) {
    snr_callback_ = callback;
}

void WelleIoBridge::setSignalPresenceCallback(rust::Fn<void(bool)> callback) {
    signal_presence_callback_ = callback;
}

void WelleIoBridge::setServiceDetectedCallback(rust::Fn<void(uint32_t, rust::String)> callback) {
    service_detected_callback_ = callback;
}

void WelleIoBridge::setNewAudioCallback(rust::Fn<void(rust::Vec<int16_t>, int32_t, bool)> callback) {
    new_audio_callback_ = callback;
}

void WelleIoBridge::setNewDynamicLabelCallback(rust::Fn<void(rust::String)> callback) {
    new_dynamic_label_callback_ = callback;
}

void WelleIoBridge::setMotCallback(rust::Fn<void(const mot_file_t&)> callback) {
    mot_callback_ = callback;
}

void WelleIoBridge::onNewAudio(std::vector<int16_t>&& audioData, int sampleRate, const std::string& mode) {
    if (!new_audio_callback_) return;
    rust::Vec<int16_t> data;
    for (int16_t sample : audioData) {
        data.push_back(sample);
    }
    (*new_audio_callback_)(std::move(data), sampleRate, mode == "stereo");
}

void WelleIoBridge::onNewDynamicLabel(const std::string& label) {
    if (!new_dynamic_label_callback_) return;
    (*new_dynamic_label_callback_)(rust::String(label));
}

void WelleIoBridge::onServiceDetected(uint32_t sId) {
    if (!service_detected_callback_ || !receiver_) return;
    Service service = receiver_->getService(sId);
    (*service_detected_callback_)(service.serviceId, rust::String(service.serviceLabel.utf8_label()));
}

void WelleIoBridge::onSNR(float snr) {
    if (!snr_callback_) return;
    (*snr_callback_)(snr);
}

void WelleIoBridge::onFrequencyCorrectorChange(int fine, int coarse) {}
void WelleIoBridge::onSyncChange(char isSync) {}
void WelleIoBridge::onSignalPresence(bool isSignal) {
    if (!signal_presence_callback_) return;
    (*signal_presence_callback_)(isSignal);
}
void WelleIoBridge::onNewEnsemble(uint16_t eId) {}
void WelleIoBridge::onSetEnsembleLabel(DabLabel& label) {}
void WelleIoBridge::onDateTimeUpdate(const dab_date_time_t& dateTime) {}
void WelleIoBridge::onFIBDecodeSuccess(bool crcCheckOk, const uint8_t* fib) {}
void WelleIoBridge::onNewImpulseResponse(std::vector<float>&& data) {}
void WelleIoBridge::onConstellationPoints(std::vector<DSPCOMPLEX>&& data) {}
void WelleIoBridge::onNewNullSymbol(std::vector<DSPCOMPLEX>&& data) {}
void WelleIoBridge::onTIIMeasurement(tii_measurement_t&& m) {}
void WelleIoBridge::onMessage(message_level_t level, const std::string& text, const std::string& text2) {}
void WelleIoBridge::onFrameErrors(int frameErrors) {}
void WelleIoBridge::onRsErrors(bool uncorrectedErrors, int numCorrectedErrors) {}
void WelleIoBridge::onAacErrors(int aacErrors) {}

void WelleIoBridge::onMOT(const mot_file_t& mot_file) {
    if (!mot_callback_) return;
    if (mot_file.content_sub_type != 0x01 && mot_file.content_sub_type != 0x03) return;

    (*mot_callback_)(mot_file);
}

void WelleIoBridge::onPADLengthError(size_t announced_xpad_len, size_t xpad_len) {}

std::unique_ptr<WelleIoBridge> new_welle_io_bridge() {
    return std::make_unique<WelleIoBridge>();
}
