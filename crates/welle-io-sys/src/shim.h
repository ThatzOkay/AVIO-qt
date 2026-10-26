#pragma once

// C:\mingw64's bundled headers don't declare these C11 additions, but its
// libstdc++ assumes they exist and fails to compile <cstdlib>/<ctime>.
#if defined(__MINGW32__)
struct timespec;
extern "C" {
[[noreturn]] void quick_exit(int) noexcept;
int at_quick_exit(void (*)()) noexcept;
int timespec_get(struct timespec*, int);
}
#endif

#include <cstddef>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <vector>
#include "rust/cxx.h"
#include "radio-controller.h"
#include "radio-receiver.h"

class WelleIoBridge : public RadioControllerInterface, public ProgrammeHandlerInterface {
public:
    WelleIoBridge();
    int32_t ping() const;

    // Set once a RadioReceiver exists, so onServiceDetected can look up the
    // service's label. Pass nullptr on stop.
    void setReceiver(RadioReceiver* receiver);

    // Callback registration; the corresponding on* handler below invokes
    // whichever callback was last registered here (nullptr means no-op).
    void setSnrCallback(rust::Fn<void(float)> callback);
    void setSignalPresenceCallback(rust::Fn<void(bool)> callback);
    void setServiceDetectedCallback(rust::Fn<void(uint32_t, rust::String)> callback);
    void setNewAudioCallback(rust::Fn<void(rust::Vec<int16_t>, int32_t, bool)> callback);
    void setNewDynamicLabelCallback(rust::Fn<void(rust::String)> callback);
    void setMotCallback(rust::Fn<void(const mot_file_t&)> callback);

    // RadioControllerInterface
    void onSNR(float snr) override;
    void onFrequencyCorrectorChange(int fine, int coarse) override;
    void onSyncChange(char isSync) override;
    void onSignalPresence(bool isSignal) override;
    void onServiceDetected(uint32_t sId) override;
    void onNewEnsemble(uint16_t eId) override;
    void onSetEnsembleLabel(DabLabel& label) override;
    void onDateTimeUpdate(const dab_date_time_t& dateTime) override;
    void onFIBDecodeSuccess(bool crcCheckOk, const uint8_t* fib) override;
    void onNewImpulseResponse(std::vector<float>&& data) override;
    void onConstellationPoints(std::vector<DSPCOMPLEX>&& data) override;
    void onNewNullSymbol(std::vector<DSPCOMPLEX>&& data) override;
    void onTIIMeasurement(tii_measurement_t&& m) override;
    void onMessage(message_level_t level, const std::string& text,
                   const std::string& text2 = std::string()) override;

    // ProgrammeHandlerInterface
    void onFrameErrors(int frameErrors) override;
    void onNewAudio(std::vector<int16_t>&& audioData, int sampleRate, const std::string& mode) override;
    void onRsErrors(bool uncorrectedErrors, int numCorrectedErrors) override;
    void onAacErrors(int aacErrors) override;
    void onNewDynamicLabel(const std::string& label) override;
    void onMOT(const mot_file_t& mot_file) override;
    void onPADLengthError(size_t announced_xpad_len, size_t xpad_len) override;

private:
    int32_t counter_;
    RadioReceiver* receiver_ = nullptr;

    std::optional<rust::Fn<void(float)>> snr_callback_;
    std::optional<rust::Fn<void(bool)>> signal_presence_callback_;
    std::optional<rust::Fn<void(uint32_t, rust::String)>> service_detected_callback_;
    std::optional<rust::Fn<void(rust::Vec<int16_t>, int32_t, bool)>> new_audio_callback_;
    std::optional<rust::Fn<void(rust::String)>> new_dynamic_label_callback_;
    std::optional<rust::Fn<void(const mot_file_t&)>> mot_callback_;
};

std::unique_ptr<WelleIoBridge> new_welle_io_bridge();
